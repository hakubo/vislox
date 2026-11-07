//! Values with their unit (#79): a port's constant shown as Lox Logic writes
//! it (`Time: 30min`, `Delay: 3h`), and a constant typed with a unit
//! (`30min`, `250ms`, `70%`) checked against the port's documented unit
//! before lox sees it.
//!
//! The rule is lox's (`docs/language.md`, "unit suffix"): a port documented
//! in milliseconds, seconds or minutes is written with the largest of `h`,
//! `min`, `s`, `ms` that gives a whole number, and only when it compiles
//! back to the same digits; zero and every other port stay bare. A time
//! suffix scales into the port's own time unit; `%`, `K` and `°` bind only
//! where the port is in that unit, or on an undocumented port.

use loxone_config::connectors::{self, PortUnit};
use loxone_config::logic::{ArgItem, Binding, BindingKind, Item, Module, Value};

/// The suffixes Lox Logic takes, each with its size in milliseconds (a
/// time) or none (an annotation).
const SUFFIXES: &[(&str, Option<u128>)] = &[
    ("ms", Some(1)),
    ("s", Some(1000)),
    ("min", Some(60_000)),
    ("h", Some(3_600_000)),
    ("%", None),
    ("K", None),
    ("°", None),
];

/// Milliseconds per unit of a port documented in a time unit.
fn per_ms(u: PortUnit) -> Option<u128> {
    match u {
        PortUnit::Milliseconds => Some(1),
        PortUnit::Seconds => Some(1000),
        PortUnit::Minutes => Some(60_000),
        _ => None,
    }
}

/// `-?digits(.digits)?` as (negative, mantissa, digits after the point).
fn decimal(s: &str) -> Option<(bool, u128, u32)> {
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (int, frac) = rest.split_once('.').unwrap_or((rest, ""));
    let ok = |p: &str| p.bytes().all(|b| b.is_ascii_digit());
    if int.is_empty() || !ok(int) || !ok(frac) || (rest.contains('.') && frac.is_empty()) {
        return None;
    }
    let m: u128 = format!("{int}{frac}").parse().ok()?;
    Some((neg, m, u32::try_from(frac.len()).ok()?))
}

/// `n / den` as the shortest exact decimal literal, `None` when it is not
/// one (`den` is a power of ten times a factor of 60).
fn render(neg: bool, n: u128, den: u128) -> Option<String> {
    let mut scale = 0u32;
    let mut num = n;
    // Multiply up until it divides, at most 6 decimal digits.
    while !num.is_multiple_of(den) {
        if scale == 6 {
            return None;
        }
        num = num.checked_mul(10)?;
        scale += 1;
    }
    let digits = (num / den).to_string();
    let s = if scale == 0 {
        digits
    } else {
        let p = scale as usize;
        let padded = format!("{digits:0>width$}", width = p + 1);
        let (i, f) = padded.split_at(padded.len() - p);
        let f = f.trim_end_matches('0');
        if f.is_empty() {
            i.to_string()
        } else {
            format!("{i}.{f}")
        }
    };
    Some(if neg && s != "0" { format!("-{s}") } else { s })
}

/// The constant `raw` (a `Def=` value) of port `key` of `ty`, written as Lox
/// Logic writes it: `1800` on a seconds port is `30min`. Anything else (no
/// time unit documented, zero, no suffix that gives back `raw` exactly)
/// stays as it is.
pub fn shown(ty: &str, key: &str, raw: &str) -> String {
    suffixed(ty, key, raw).unwrap_or_else(|| raw.to_string())
}

fn suffixed(ty: &str, key: &str, raw: &str) -> Option<String> {
    let per = per_ms(connectors::port_unit(ty, key)?)?;
    let (neg, m, frac) = decimal(raw)?;
    let den = 10u128.checked_pow(frac)?;
    let scaled = m.checked_mul(per)?;
    if scaled == 0 || scaled % den != 0 {
        return None;
    }
    let ms = scaled / den;
    for (suffix, size) in [
        ("h", 3_600_000u128),
        ("min", 60_000),
        ("s", 1000),
        ("ms", 1),
    ] {
        if ms % size != 0 {
            continue;
        }
        let n = ms / size;
        // Back into the port's unit: exactly `raw`, or no suffix.
        if render(neg, n * size, per).as_deref() == Some(raw) {
            return Some(format!("{}{n}{suffix}", if neg { "-" } else { "" }));
        }
    }
    None
}

/// A constant typed for port `key` of `ty`: a plain number, or one with a
/// unit suffix the port takes (`30min` on a seconds port, `70%` on a percent
/// port or an undocumented one). The error says why, without the port's name.
pub fn port_value(ty: &str, key: &str, v: &str) -> Result<Value, String> {
    let v = v.trim();
    let at = v
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(v.len());
    let (n, suffix) = v.split_at(at);
    if suffix.is_empty() {
        return crate::edit::number(n).map(Value::Number);
    }
    let Some((_, size)) = SUFFIXES.iter().find(|(s, _)| *s == suffix) else {
        // Not a number at all, or one with a comma: say what is wrong.
        if n.is_empty() || suffix.starts_with([',', '.', '-']) {
            crate::edit::number(v)?;
        }
        return Err(format!(
            "`{v}`: write a number, or one with a unit (ms, s, min, h, %, K or °)"
        ));
    };
    crate::edit::number(n)
        .map_err(|_| format!("`{v}` is not a number with a unit (like 30min)"))?;
    let unit = connectors::port_unit(ty, key);
    match (size, unit) {
        (_, None) => {}
        (Some(size), Some(u)) => {
            let Some(per) = per_ms(u) else {
                return Err(format!(
                    "`{v}`: the port is in {}, not a time",
                    u.describe()
                ));
            };
            // It must be a whole number of the port's own unit, as lox
            // writes it (`10s` on a minutes port is not).
            // Checked: a number too long to scale is refused, never a panic.
            let too_long = || format!("`{v}` has too many digits");
            let (neg, m, frac) = decimal(n).ok_or_else(too_long)?;
            let den = 10u128
                .checked_pow(frac)
                .and_then(|d| d.checked_mul(per))
                .ok_or_else(too_long)?;
            let ms = m.checked_mul(*size).ok_or_else(too_long)?;
            if render(neg, ms, den).is_none() {
                return Err(format!("`{v}` is not an exact number of {}", u.describe()));
            }
        }
        (None, Some(u)) => {
            let fits = matches!(
                (suffix, u),
                ("%", PortUnit::Percent) | ("K", PortUnit::Kelvin) | ("°", PortUnit::Degrees)
            );
            if !fits {
                return Err(format!("`{v}`: the port is in {}", u.describe()));
            }
        }
    }
    // Read back with lox's own parser, so it is spelled the way lox keeps it.
    lox_value(&format!("{n}{suffix}")).ok_or_else(|| format!("`{v}` is not a number with a unit"))
}

/// A number of port or setting `key` of `ty` in its own documented unit:
/// a plain number as it is, a time with a suffix scaled into it (`5s` on a
/// seconds setting is `5`, `46s` on a milliseconds one `46000`). `None`
/// when it is not one exactly (no time unit documented, another kind of
/// unit, not a whole number of it).
pub fn in_own_unit(ty: &str, key: &str, v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => Some(n.clone()),
        Value::Unit { number, unit } if unit.is_time() => {
            let per = per_ms(connectors::port_unit(ty, key)?)?;
            let size = SUFFIXES
                .iter()
                .find(|(s, _)| *s == unit.suffix())
                .and_then(|(_, z)| *z)?;
            let (neg, m, frac) = decimal(number)?;
            let den = 10u128.checked_pow(frac)?.checked_mul(per)?;
            render(neg, m.checked_mul(size)?, den)
        }
        _ => None,
    }
}

/// `text` as lox's parser reads it in an argument (`30min` → a unit value),
/// `None` when it is not one value.
pub fn lox_value(text: &str) -> Option<Value> {
    let m = Module::parse_fragment(&format!("vx_u = Monoflop(Time: {text})\n")).ok()?;
    m.items.into_iter().find_map(|i| match i {
        Item::Block(b) => b.args.into_iter().find_map(|a| match a {
            ArgItem::Binding(Binding {
                kind: BindingKind::Param(v),
                ..
            }) => Some(v),
            _ => None,
        }),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_port_shows_the_largest_whole_unit() {
        // Monoflop's Time is documented in seconds.
        assert_eq!(shown("Monoflop", "Time", "1800"), "30min");
        assert_eq!(shown("Monoflop", "Time", "10800"), "3h");
        assert_eq!(shown("Monoflop", "Time", "58"), "58s");
        assert_eq!(shown("Monoflop", "Time", "0.5"), "500ms");
        assert_eq!(shown("Monoflop", "Time", "0"), "0");
        assert_eq!(shown("Monoflop", "Time", "1.0"), "1.0");
        assert_eq!(shown("Monoflop", "Time", "0.0001"), "0.0001");
        // An undocumented port, or one in another unit, stays bare.
        assert_eq!(shown("Add", "I1", "1800"), "1800");
    }

    #[test]
    fn a_typed_unit_must_fit_the_port() {
        assert_eq!(
            port_value("Monoflop", "Time", "30min").unwrap().to_string(),
            "30min"
        );
        assert_eq!(
            port_value("Monoflop", "Time", " 12 ").unwrap(),
            Value::Number("12".into())
        );
        assert!(port_value("Monoflop", "Time", "70%").is_err());
        assert!(port_value("Monoflop", "Time", "5parsecs").is_err());
        assert!(port_value("Monoflop", "Time", "1,5h").is_err());
        // An undocumented port takes any unit, as lox reads it.
        assert!(port_value("Add", "I1", "70%").is_ok());
        assert!(port_value("Add", "I1", "2h").is_ok());
        // Too long to scale: refused, never a panic or a wrong value.
        let long = format!("{}s", "9".repeat(40));
        assert!(
            port_value("Monoflop", "Time", &long)
                .unwrap_err()
                .contains("too many digits")
        );
        let frac = format!("1.{}1s", "0".repeat(40));
        assert!(
            port_value("Monoflop", "Time", &frac)
                .unwrap_err()
                .contains("too many digits")
        );
        let frac = format!("0.{}1min", "0".repeat(30));
        assert!(port_value("Monoflop", "Time", &frac).is_err());
    }
}
