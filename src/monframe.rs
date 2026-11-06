//! The Debug-Monitor's UDP frames, as a Miniserver sends them to port 7777.
//!
//! The layout follows sarnau's notes (`Code/LoxoneMonitorServer.py` in
//! Inside-The-Loxone-Miniserver, firmware 10.x). A datagram holds one or more
//! frames, padded with `1F 1F`:
//!
//! - bytes 0–1: `0xFA1F` (little endian), 2–3: the frame's size;
//! - byte 4: XOR of bytes 5…size, the checksum;
//! - byte 5: the number of parameters; 6–7: the string ID; 8–9: an event
//!   counter; 10–11: milliseconds; 12–15: Loxone time (seconds since
//!   2009-01-01); 16–19: the source IP; 20–23: used memory; 24–27: unknown.
//!
//! A string ID below `0xFFF0` is a log line: its format string is in the
//! Miniserver's string table (`ENG.xml`), and typed parameters follow.
//! `0xFFFF` is the system statistics, `0xFFFE` the Miniserver's own I/O and
//! `0xFFF0`–`0xFFFD` the task list.

use serde::Serialize;
use std::collections::HashMap;
use std::net::Ipv4Addr;

pub const MAGIC: u16 = 0xFA1F;
const PAD: u16 = 0x1F1F;
pub const HEADER: usize = 28;
pub const STATS: u16 = 0xFFFF;
pub const TASKS: u16 = 0xFFF0;

/// The frames of one datagram whose checksum holds. Padding is skipped; a
/// frame cut short, or bytes that are not a frame, end the datagram.
pub fn frames(mut data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    while data.len() >= 2 {
        match u16::from_le_bytes([data[0], data[1]]) {
            PAD => data = &data[2..],
            MAGIC if data.len() >= 5 => {
                let size = u16::from_le_bytes([data[2], data[3]]) as usize;
                if size < HEADER || size > data.len() {
                    break;
                }
                let f = &data[..size];
                if f[5..].iter().fold(0u8, |x, b| x ^ b) == f[4] {
                    out.push(f);
                }
                data = &data[size..];
            }
            _ => break,
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub params: u8,
    pub string_id: u16,
    pub counter: u16,
    pub ms: u16,
    /// Seconds since 2009-01-01 00:00, the Miniserver's clock.
    pub time: u32,
    pub source: Ipv4Addr,
}

impl Header {
    pub fn parse(f: &[u8]) -> Option<Header> {
        if f.len() < HEADER {
            return None;
        }
        let u16_at = |i: usize| u16::from_le_bytes([f[i], f[i + 1]]);
        Some(Header {
            params: f[5],
            string_id: u16_at(6),
            counter: u16_at(8),
            ms: u16_at(10),
            time: u32::from_le_bytes([f[12], f[13], f[14], f[15]]),
            source: Ipv4Addr::new(f[16], f[17], f[18], f[19]),
        })
    }

    /// `2026-11-06 10:57:23.700`, as `lox log` writes its times.
    pub fn at(&self) -> String {
        let base = chrono::NaiveDate::from_ymd_opt(2009, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let t = base
            + chrono::TimeDelta::seconds(self.time as i64)
            + chrono::TimeDelta::milliseconds((self.ms % 1000) as i64);
        t.format("%Y-%m-%d %H:%M:%S%.3f").to_string()
    }
}

/// A typed parameter of a log line.
#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    Str(String),
    Buf(Vec<u8>),
    Int(u32),
    Ip(Ipv4Addr),
    Mac([u8; 6]),
    Tcp(u32, u8),
}

impl Param {
    fn text(&self) -> String {
        match self {
            Param::Str(s) => s.clone(),
            Param::Buf(b) => hex::encode(b),
            Param::Int(n) => n.to_string(),
            Param::Ip(ip) => ip.to_string(),
            Param::Mac(m) => m
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(":"),
            Param::Tcp(ack, flags) => format!("{ack} {flags:02x}"),
        }
    }
}

/// The parameters after the header. An unknown type ends the list.
pub fn params(f: &[u8], count: u8) -> Vec<Param> {
    let mut out = Vec::new();
    let mut i = HEADER;
    for _ in 0..count {
        let Some(&ty) = f.get(i) else { break };
        i += 1;
        let rest = &f[i.min(f.len())..];
        let (p, used) = match ty {
            0x01 => {
                let n = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
                (
                    Param::Str(String::from_utf8_lossy(&rest[..n]).into_owned()),
                    (n + 1).min(rest.len()),
                )
            }
            0x02 if rest.len() >= 2 => {
                let n = (u16::from_le_bytes([rest[0], rest[1]]) as usize).min(rest.len() - 2);
                (Param::Buf(rest[2..2 + n].to_vec()), 2 + n)
            }
            0x03 if rest.len() >= 4 => (
                Param::Int(u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]])),
                4,
            ),
            0x04 if rest.len() >= 4 => (
                Param::Ip(Ipv4Addr::new(rest[0], rest[1], rest[2], rest[3])),
                4,
            ),
            0x05 if rest.len() >= 6 => {
                let mut m = [0; 6];
                m.copy_from_slice(&rest[..6]);
                (Param::Mac(m), 6)
            }
            0x06 if rest.len() >= 5 => (
                Param::Tcp(
                    u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]),
                    rest[4],
                ),
                5,
            ),
            _ => break,
        };
        out.push(p);
        i += used;
    }
    out
}

/// The format strings of the Miniserver's string table (`ENG.xml`:
/// `<String IDV="1234" Text="…"/>`).
#[derive(Default)]
pub struct Strings(HashMap<u16, String>);

impl Strings {
    pub fn parse(xml: &str) -> Strings {
        let mut m = HashMap::new();
        for line in xml.split("<String ").skip(1) {
            let attr = |name: &str| {
                let start = line.find(&format!("{name}=\""))? + name.len() + 2;
                let end = line[start..].find('"')? + start;
                Some(unescape(&line[start..end]))
            };
            if let (Some(id), Some(text)) = (attr("IDV"), attr("Text"))
                && let Ok(id) = id.parse::<u16>()
            {
                m.insert(id, text);
            }
        }
        Strings(m)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn get(&self, id: u16) -> Option<&str> {
        self.0.get(&id).map(String::as_str)
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The text of a log line: its format string with the parameters filled in
/// (`%s`, `%d`, `%u`, `%x`, `%08X`, `%ld`, …; parameters left over are
/// added at the end). Without a format string, the parameters, separated
/// by spaces: current firmware sends most lines as one string parameter.
pub fn format(fmt: Option<&str>, params: &[Param]) -> String {
    let Some(fmt) = fmt else {
        return params
            .iter()
            .map(Param::text)
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();
    };
    let mut out = String::new();
    let mut it = params.iter();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut spec = String::new();
        let conv = loop {
            match chars.next() {
                None => break None,
                Some(c) if "-+ #0123456789.lhzjt".contains(c) => spec.push(c),
                Some(c) => break Some(c),
            }
        };
        let Some(conv) = conv else {
            out.push('%');
            out.push_str(&spec);
            break;
        };
        if conv == '%' {
            out.push('%');
            continue;
        }
        let Some(p) = it.next() else {
            out.push('%');
            out.push_str(&spec);
            out.push(conv);
            continue;
        };
        let digits: String = spec
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let (width, _) = digits.split_once('.').unwrap_or((&digits, ""));
        let zero = width.starts_with('0');
        let width: usize = width.parse().unwrap_or(0);
        let left = spec.contains('-');
        let body = match (conv, p) {
            ('x', Param::Int(n)) => format!("{n:x}"),
            ('X', Param::Int(n)) => format!("{n:X}"),
            ('d' | 'i', Param::Int(n)) => (*n as i32).to_string(),
            ('c', Param::Int(n)) => char::from_u32(*n).map(String::from).unwrap_or_default(),
            (_, p) => p.text(),
        };
        let pad = width.saturating_sub(body.chars().count());
        if left {
            out.push_str(&body);
            out.push_str(&" ".repeat(pad));
        } else {
            out.push_str(&(if zero { "0" } else { " " }).repeat(pad));
            out.push_str(&body);
        }
    }
    for p in it {
        out.push(' ');
        out.push_str(&p.text());
    }
    out.trim_end().to_string()
}

/// The system statistics (`0xFFFF`, version 3).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stats {
    /// CPU load, %.
    pub cpu: f32,
    /// Heap in use and its maximum, kB.
    pub heap_kb: u32,
    pub max_kb: u32,
    /// Time since boot, ms.
    pub uptime_ms: u32,
    pub lan_tx_errors: u32,
    pub link_errors: u32,
    pub link_tx_errors: u32,
    pub link_rx_errors: u32,
}

impl Stats {
    pub fn parse(f: &[u8]) -> Option<Stats> {
        let body = f.get(HEADER..HEADER + 4 + 32 * 4)?;
        if u16::from_le_bytes([body[2], body[3]]) != 3 {
            return None;
        }
        let n = |i: usize| {
            let o = 4 + i * 4;
            u32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]])
        };
        let (idle, cycle) = (n(4), n(5));
        let cpu = if cycle == 0 {
            0.0
        } else {
            (100.0 - idle as f32 * 100.0 / cycle as f32).clamp(0.0, 100.0)
        };
        Some(Stats {
            cpu,
            heap_kb: n(3).div_ceil(1024),
            max_kb: n(2).div_ceil(1024),
            uptime_ms: n(8),
            lan_tx_errors: n(13),
            link_errors: n(25),
            link_tx_errors: n(27),
            link_rx_errors: n(28),
        })
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A frame as a Miniserver would send it.
    pub fn frame(
        string_id: u16,
        counter: u16,
        time: u32,
        ms: u16,
        body: &[u8],
        params: u8,
    ) -> Vec<u8> {
        let size = (HEADER + body.len()) as u16;
        let mut f = Vec::new();
        f.extend(MAGIC.to_le_bytes());
        f.extend(size.to_le_bytes());
        f.push(0);
        f.push(params);
        f.extend(string_id.to_le_bytes());
        f.extend(counter.to_le_bytes());
        f.extend(ms.to_le_bytes());
        f.extend(time.to_le_bytes());
        f.extend([192, 168, 1, 120]);
        f.extend(123_456u32.to_le_bytes());
        f.extend(0u32.to_le_bytes());
        f.extend(body);
        f[4] = f[5..].iter().fold(0, |x, b| x ^ b);
        f
    }

    pub fn string(s: &str) -> Vec<u8> {
        let mut b = vec![0x01];
        b.extend(s.as_bytes());
        b.push(0);
        b
    }

    pub fn stats(idle: u32, cycle: u32, heap: u32, max: u32) -> Vec<u8> {
        let mut b = vec![0, 0];
        b.extend(3u16.to_le_bytes());
        let mut v = [0u32; 32];
        v[2] = max;
        v[3] = heap;
        v[4] = idle;
        v[5] = cycle;
        v[8] = 90_000;
        v[25] = 2;
        for x in v {
            b.extend(x.to_le_bytes());
        }
        b
    }

    #[test]
    fn frames_split_padding_and_bad_checksums() {
        let a = frame(1, 1, 0, 0, &string("one"), 1);
        let mut b = frame(1, 2, 0, 0, &string("two"), 1);
        let mut data = a.clone();
        data.extend([0x1F, 0x1F]);
        data.extend(&b);
        data.extend([0x1F, 0x1F, 0x1F, 0x1F]);
        assert_eq!(frames(&data), vec![&a[..], &b[..]]);
        // A broken checksum drops that frame only.
        b[4] ^= 0xFF;
        let mut data = b.clone();
        data.extend(&a);
        assert_eq!(frames(&data), vec![&a[..]]);
        // Cut short, or not a frame: nothing, and no endless loop.
        assert!(frames(&a[..a.len() - 1]).is_empty());
        assert!(frames(b"hello world").is_empty());
        let mut huge = a.clone();
        huge[2] = 0xFF;
        huge[3] = 0xFF;
        assert!(frames(&huge).is_empty());
    }

    #[test]
    fn the_header_has_the_time_and_source() {
        // 2026-11-06 10:57:23 is 563194643 s after 2009-01-01.
        let secs = (chrono::NaiveDate::from_ymd_opt(2026, 11, 6)
            .unwrap()
            .and_hms_opt(10, 57, 23)
            .unwrap()
            - chrono::NaiveDate::from_ymd_opt(2009, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap())
        .num_seconds() as u32;
        let f = frame(42, 7, secs, 700, &string("x"), 1);
        let h = Header::parse(&f).unwrap();
        assert_eq!(h.string_id, 42);
        assert_eq!(h.counter, 7);
        assert_eq!(h.source, Ipv4Addr::new(192, 168, 1, 120));
        assert_eq!(h.at(), "2026-11-06 10:57:23.700");
    }

    #[test]
    fn every_parameter_type_is_read() {
        let mut body = string("PRG start");
        body.extend([0x02, 2, 0, 0xAB, 0xCD]);
        body.push(0x03);
        body.extend(4711u32.to_le_bytes());
        body.extend([0x04, 192, 168, 1, 82]);
        body.extend([0x05, 0x50, 0x4F, 0x94, 0xA0, 0xE4, 0x54]);
        body.push(0x06);
        body.extend(99u32.to_le_bytes());
        body.push(0x12);
        let f = frame(1, 1, 0, 0, &body, 6);
        assert_eq!(
            params(&f, 6),
            vec![
                Param::Str("PRG start".into()),
                Param::Buf(vec![0xAB, 0xCD]),
                Param::Int(4711),
                Param::Ip(Ipv4Addr::new(192, 168, 1, 82)),
                Param::Mac([0x50, 0x4F, 0x94, 0xA0, 0xE4, 0x54]),
                Param::Tcp(99, 0x12),
            ]
        );
        // More parameters announced than there are: what is there.
        assert_eq!(params(&f, 9).len(), 6);
        // An unknown type ends the list.
        let f = frame(1, 1, 0, 0, &[0x09, 1, 2], 2);
        assert!(params(&f, 2).is_empty());
    }

    #[test]
    fn format_strings_take_the_parameters() {
        let p = [
            Param::Str("Relay 1".into()),
            Param::Int(0x0bdbbf08),
            Param::Int(5),
        ];
        assert_eq!(
            format(Some("PRG start %s serial %08x after %d s"), &p),
            "PRG start Relay 1 serial 0bdbbf08 after 5 s"
        );
        assert_eq!(format(Some("%-6s|%3d%%"), &p[..1]), "Relay 1|%3d%");
        assert_eq!(format(Some("only %s"), &p), "only Relay 1 198950664 5");
        assert_eq!(format(None, &p[..1]), "Relay 1");
        assert_eq!(
            format(Some("%s at %s"), &[Param::Ip(Ipv4Addr::new(10, 0, 0, 1))]),
            "10.0.0.1 at %s"
        );
        assert_eq!(
            format(Some("%5d|%-3d|"), &[Param::Int(42), Param::Int(1)]),
            "   42|1  |"
        );
    }

    #[test]
    fn the_string_table_is_read_from_eng_xml() {
        let s = Strings::parse(
            r#"<?xml version="1.0"?><Strings>
  <String IDV="560" Text="Thread%d"/>
  <String IDV="1200" Text="PRG start &quot;%s&quot; &amp; more"/>
  <String Text="no id"/>
</Strings>"#,
        );
        assert_eq!(s.len(), 2);
        assert_eq!(s.get(1200), Some("PRG start \"%s\" & more"));
        assert_eq!(s.get(1), None);
    }

    #[test]
    fn system_stats_give_cpu_and_heap() {
        let f = frame(STATS, 1, 0, 0, &stats(750, 1000, 10_240, 20_000), 0);
        let s = Stats::parse(&f).unwrap();
        assert_eq!(s.cpu, 25.0);
        assert_eq!(s.heap_kb, 10);
        assert_eq!(s.max_kb, 20);
        assert_eq!(s.uptime_ms, 90_000);
        assert_eq!(s.link_errors, 2);
        // Another version, or too short: not read.
        let mut b = stats(1, 2, 3, 4);
        b[2] = 4;
        assert!(Stats::parse(&frame(STATS, 1, 0, 0, &b, 0)).is_none());
        assert!(Stats::parse(&frame(STATS, 1, 0, 0, &[0; 10], 0)).is_none());
    }
}
