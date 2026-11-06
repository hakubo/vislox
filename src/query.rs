//! Querying blocks the Miniserver does not stream (opt-in, read-only).
//!
//! The Miniserver streams only the states of blocks that are in the app.
//! `jdev/sps/io/<block uuid>/all` returns the current outputs of any block:
//!
//! ```json
//! {"LL": {"control": "dev/sps/io/<uuid>/all", "Code": "200", "value": "0.000",
//!         "output0": {"name": "R", "nr": 1, "value": 0.0},
//!         "output1": {"name": "E", "nr": 2, "value": ""}}}
//! ```
//!
//! `nr` counts the block's outputs in config order, and `name` is the label
//! Loxone Config draws (`R` for a Formula's `AQ`), not the connector key. So
//! outputs map to connectors by position when the counts match, else by a
//! name that matches the key (`Pf` → `OPf` on a meter, which leaves out its
//! weekly totals), else by a per-type table. Blocks with one value (a memory
//! flag, a mode, an actuator) answer only `value`, with its unit (`9.6°`).
//! This module builds nothing itself: `model::query_plan` picks the blocks of
//! a page, and `live.rs` sends the requests.

use loxone_config::connectors::{PortDir, builtin};
use serde_json::Value;

/// A block to query, and its connectors in config order.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub uuid: String,
    pub ty: String,
    pub title: String,
    /// `(key, connector uuid, name)`, in config order.
    pub ports: Vec<(String, String, String)>,
    /// The connectors whose values the page needs (wire sources).
    pub want: Vec<String>,
}

/// The request for a block's outputs.
pub fn command(uuid: &str) -> String {
    format!("jdev/sps/io/{uuid}/all")
}

/// Types whose answer leaves outputs out or orders them differently: the
/// connector key for each answered output, in answer order (`None`: not a
/// connector of the block). Taken from a survey of a real Miniserver
/// (firmware 17.2): the answer's labels next to the block's connectors.
fn table(ty: &str) -> Option<&'static [Option<&'static str>]> {
    Some(match ty {
        // H H2 C C2 Ah Sv F Fc ϑoa API: no error output.
        "HVACController" => &[
            Some("OutHeat1"),
            Some("OutHeat2"),
            Some("OutCool1"),
            Some("OutCool2"),
            Some("OutSecHeat"),
            Some("OutValve"),
            Some("OutFan"),
            Some("OutFilter"),
            Some("OutAverageTemp"),
            Some("OutputAPI"),
        ],
        // Pc P Pon Poff Pd Warn API: no device times.
        "PresenceDetector" => &[
            Some("OutputPresence"),
            Some("OutputActive"),
            Some("OutputOn"),
            Some("OutputOff"),
            Some("OutputOnTime"),
            Some("OutputWarn"),
            Some("OutputAPI"),
        ],
        // Op Cl Pos TPos API: no Direction, no ProtectionOut.
        "RoofWindow" => &[
            Some("OpenOut"),
            Some("CloseOut"),
            Some("Position"),
            Some("TargetPos"),
            Some("OutputAPI"),
        ],
        // Stereo LR, L, R, Sub, Play, Volume, 2C, 3C, V+, V-, BTp, API: no
        // zone favourite.
        "MusicPlayer" => &[
            Some("OutAudio"),
            Some("OutAudioL"),
            Some("OutAudioR"),
            Some("OutAudioSub"),
            Some("OutActive"),
            Some("OutVolume"),
            Some("RoomOff"),
            Some("HouseOff"),
            Some("OutVolUpExternal"),
            Some("OutVolDownExternal"),
            Some("OutBluetooth"),
            Some("OutputAPI"),
        ],
        // Op Cl Pos Wds Off Sp API: `AutoState` is not named like an output.
        "ShadeRoof" => &[
            Some("OutputUp"),
            Some("OutputDown"),
            Some("OutputPos"),
            Some("OutputSafety"),
            Some("OutputLock"),
            Some("AutoState"),
            Some("OutputAPI"),
        ],
        // Tg Op Cl Im Pos Wl API: `Light` is not named like an output.
        "Door" => &[
            Some("OutputTrigger"),
            Some("OutputOpen"),
            Some("OutputClose"),
            Some("OutputActive"),
            Some("OutputPos"),
            Some("Light"),
            Some("OutputAPI"),
        ],
        // Pt Bm P1…P7 Soc Ol API: no remaining time.
        "PowerUnit" => &[
            Some("OutPower"),
            Some("OutBackup"),
            Some("Power1"),
            Some("Power2"),
            Some("Power3"),
            Some("Power4"),
            Some("Power5"),
            Some("Power6"),
            Some("Power7"),
            Some("OutSOC"),
            Some("OutOverCurrent"),
            Some("OutputAPI"),
        ],
        // O Cv vHigh High Low vLow Max Min Avg Nv API.
        "SpotOpt" => &[
            Some("Act"),
            Some("Uc"),
            Some("vHigh"),
            Some("High"),
            Some("Low"),
            Some("vLow"),
            Some("Max"),
            Some("Min"),
            Some("Avg"),
            Some("Nv"),
            Some("OutputAPI"),
        ],
        _ => return None,
    })
}

/// Types left out: their answer does not say which connector a value
/// belongs to. A lighting controller lists its circuits (not its Mood or
/// Reset outputs), a State block its active entry, a smoke alarm nine of
/// its eleven outputs under labels that do not tell which.
pub fn unmapped(ty: &str) -> bool {
    matches!(ty, "LightController2" | "State" | "SmokeAlarm")
}

/// AutoJalousie answers named attributes instead of outputs.
const NAMED: &[(&str, &str, &str)] = &[
    ("AutoJalousie", "StateUp", "OutputUp"),
    ("AutoJalousie", "StateDown", "OutputDown"),
    ("AutoJalousie", "StatePos", "OutputPos"),
    ("AutoJalousie", "StateAutoShade", "OutputAutoShade"),
    ("AutoJalousie", "StateSafety", "OutputSafety"),
];

/// Whether a connector is an output: as `loxone_config` knows the type,
/// else by its key (`Q…`, `AQ…`, `TQ…`, `Out…`).
fn is_output(ty: &str, key: &str) -> bool {
    if key == "OutputAPI" || key == "API" {
        return true;
    }
    match builtin(ty).and_then(|s| s.iter().find(|s| s.key == key).map(|s| s.dir)) {
        Some(d) => matches!(d, PortDir::Output | PortDir::Api),
        None => ["Q", "AQ", "TQ", "Out"].iter().any(|p| key.starts_with(p)),
    }
}

/// A value as JSON: whole numbers as integers.
fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Value::from(v as i64)
    } else {
        Value::from(v)
    }
}

/// A value with an optional unit (`9.6°`, `0.0kW`, `42 %`) as a number.
/// Text that only starts with a number (`12:30`, `2026-11-06`) is not one:
/// the unit may hold no digits.
fn leading_number(s: &str) -> Option<f64> {
    let s = s.trim();
    let end = s
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || (i == 0 && (c == '-' || c == '+'))))
        .map_or(s.len(), |(i, _)| i);
    let unit = &s[end..];
    if unit.chars().any(|c| c.is_ascii_digit()) || unit.starts_with(['.', ':', '-', '/', ',']) {
        return None;
    }
    s[..end].parse().ok()
}

fn plain(v: &Value) -> Value {
    match v {
        Value::Number(n) => n.as_f64().map(number).unwrap_or(Value::Null),
        other => other.clone(),
    }
}

/// Why a query failed, from its answer.
pub fn failure(ll: &Value) -> Option<String> {
    let code = match &ll["Code"] {
        Value::String(s) => s.parse::<u16>().ok(),
        Value::Number(n) => n.as_u64().map(|n| n as u16),
        _ => ll["code"].as_u64().map(|n| n as u16),
    };
    match code {
        Some(200) => None,
        Some(c) => Some(format!("Code {c}")),
        None => Some("no Code in the answer".into()),
    }
}

/// The block's connector values from its answer (`LL`):
/// `(connector uuid, value, key, name)`.
pub fn map(b: &Block, ll: &Value) -> Vec<(String, Value, String, String)> {
    let port = |key: &str| b.ports.iter().find(|p| p.0 == key);
    let out =
        |p: &(String, String, String), v: &Value| (p.1.clone(), plain(v), p.0.clone(), p.2.clone());
    if unmapped(&b.ty) {
        return Vec::new();
    }
    let Some(obj) = ll.as_object() else {
        return Vec::new();
    };
    let mut answered: Vec<(u64, &str, &Value)> = obj
        .iter()
        .filter(|(k, _)| k.starts_with("output") && k[6..].chars().all(|c| c.is_ascii_digit()))
        .filter_map(|(_, v)| {
            Some((
                v["nr"].as_u64()?,
                v["name"].as_str().unwrap_or(""),
                &v["value"],
            ))
        })
        .collect();
    answered.sort_by_key(|a| a.0);
    let named: Vec<_> = NAMED
        .iter()
        .filter(|(ty, ..)| *ty == b.ty)
        .filter_map(|(_, attr, key)| Some(out(port(key)?, obj.get(*attr)?)))
        .collect();
    if !named.is_empty() {
        return named;
    }
    if answered.is_empty() {
        // One value: the block's main (first) output.
        let Some(v) = obj.get("value") else {
            return Vec::new();
        };
        let v = match v {
            Value::String(s) => match leading_number(s) {
                Some(n) => number(n),
                None if s.is_empty() => return Vec::new(),
                None => v.clone(),
            },
            other => plain(other),
        };
        return b
            .ports
            .iter()
            .find(|p| is_output(&b.ty, &p.0))
            .map(|p| vec![(p.1.clone(), v, p.0.clone(), p.2.clone())])
            .unwrap_or_default();
    }
    if let Some(keys) = table(&b.ty) {
        return answered
            .iter()
            .zip(keys)
            .filter_map(|((_, _, v), k)| Some(out(port((*k)?)?, v)))
            .collect();
    }
    let outs: Vec<_> = b.ports.iter().filter(|p| is_output(&b.ty, &p.0)).collect();
    if outs.len() == answered.len() {
        return outs
            .iter()
            .zip(&answered)
            .map(|(p, (_, _, v))| out(p, v))
            .collect();
    }
    // Counts differ: only labels that name the connector.
    answered
        .iter()
        .filter(|(_, name, _)| !name.is_empty())
        .filter_map(|(_, name, v)| {
            let p = outs.iter().find(|p| {
                let k = p.0.as_str();
                k == *name
                    || ["O", "Out", "Output"]
                        .iter()
                        .any(|pre| k.strip_prefix(pre) == Some(name))
            })?;
            Some(out(p, v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn block(ty: &str, keys: &[&str]) -> Block {
        Block {
            uuid: "b".into(),
            ty: ty.into(),
            title: "B".into(),
            ports: keys
                .iter()
                .map(|k| (k.to_string(), format!("u-{k}"), k.to_string()))
                .collect(),
            want: Vec::new(),
        }
    }

    fn values(b: &Block, ll: Value) -> Vec<(String, Value)> {
        map(b, &ll).into_iter().map(|(u, v, ..)| (u, v)).collect()
    }

    fn outputs(xs: &[(&str, Value)]) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("Code".into(), json!("200"));
        m.insert("value".into(), json!("0"));
        for (i, (name, v)) in xs.iter().enumerate() {
            m.insert(
                format!("output{i}"),
                json!({ "name": name, "nr": i + 1, "value": v }),
            );
        }
        Value::Object(m)
    }

    #[test]
    fn outputs_map_by_position_when_the_counts_match() {
        // A Formula's labels (R, E) are not its keys (AQ, TQ).
        let f = block("Formula", &["Input1", "Input2", "AQ", "TQ"]);
        let ll = outputs(&[("R", json!(21.5)), ("E", json!(""))]);
        assert_eq!(
            values(&f, ll),
            vec![("u-AQ".into(), json!(21.5)), ("u-TQ".into(), json!(""))]
        );
        // A type loxone_config does not know: outputs by their keys. The
        // answer's numbers decide the order, not its keys.
        let r = block(
            "Radio",
            &[
                "InputTrigger 1",
                "Reset",
                "Mode",
                "Q1",
                "Q2",
                "AQ",
                "OutputAPI",
            ],
        );
        let mut ll = outputs(&[
            ("O1", json!(0)),
            ("O2", json!(1)),
            ("N", json!(2.0)),
            ("API", json!("")),
        ]);
        let o0 = ll["output0"].clone();
        ll["output0"] = ll["output2"].clone();
        ll["output2"] = o0;
        let v = values(&r, ll);
        assert_eq!(v[1], ("u-Q2".into(), json!(1)));
        assert_eq!(v[2], ("u-AQ".into(), json!(2)));
    }

    #[test]
    fn a_meter_maps_by_name_without_its_weekly_totals() {
        let m = block(
            "MeterAbsUni",
            &[
                "Mr",
                "R",
                "Mro",
                "OPf",
                "OMr",
                "ORd",
                "ORld",
                "ORw",
                "ORlw",
                "ORm",
                "OutputAPI",
            ],
        );
        let ll = outputs(&[
            ("Pf", json!(0.5)),
            ("Mr", json!(122.942)),
            ("Rd", json!(1.0)),
            ("Rld", json!(2.0)),
            ("Rm", json!(3.0)),
            ("API", json!("")),
        ]);
        let v = values(&m, ll);
        assert!(v.contains(&("u-OPf".into(), json!(0.5))));
        assert!(v.contains(&("u-OMr".into(), json!(122.942))));
        assert!(v.contains(&("u-ORm".into(), json!(3))));
        assert!(!v.iter().any(|(u, _)| u == "u-ORw"));
    }

    #[test]
    fn a_table_maps_types_that_leave_outputs_out() {
        let w = block(
            "RoofWindow",
            &[
                "Toggle",
                "OpenOut",
                "CloseOut",
                "Position",
                "TargetPos",
                "Direction",
                "ProtectionOut",
                "OutputAPI",
            ],
        );
        let ll = outputs(&[
            ("Op", json!(1)),
            ("Cl", json!(0)),
            ("Pos", json!(0.25)),
            ("TPos", json!(0.5)),
            ("API", json!("")),
        ]);
        let v = values(&w, ll);
        assert_eq!(v[2], ("u-Position".into(), json!(0.25)));
        assert_eq!(v[3], ("u-TargetPos".into(), json!(0.5)));
        assert_eq!(v.len(), 5);
    }

    #[test]
    fn one_value_goes_to_the_first_output_without_its_unit() {
        let s = block("SysVar", &["AQ", "Q"]);
        assert_eq!(
            values(&s, json!({"Code": "200", "value": "9.6°"})),
            vec![("u-AQ".into(), json!(9.6))]
        );
        let m = block("Mode", &["I", "Q"]);
        assert_eq!(
            values(&m, json!({"Code": "200", "value": "1"})),
            vec![("u-Q".into(), json!(1))]
        );
        assert_eq!(values(&m, json!({"Code": "200", "value": ""})), vec![]);
        assert_eq!(leading_number("-3.5kW"), Some(-3.5));
        assert_eq!(leading_number("on"), None);
        assert_eq!(leading_number("42 %"), Some(42.0));
        // Text that starts with a number stays text.
        for t in ["12:30", "2026-11-06", "1.2.3", "3/4", "1,5"] {
            assert_eq!(leading_number(t), None, "{t}");
        }
        let t = block("TextVar", &["TQ"]);
        assert_eq!(
            values(&t, json!({"Code": "200", "value": "12:30"})),
            vec![("u-TQ".into(), json!("12:30"))]
        );
    }

    #[test]
    fn named_states_and_left_out_types() {
        let j = block(
            "AutoJalousie",
            &["InputTrigger", "OutputUp", "OutputDown", "OutputPos"],
        );
        let v = values(
            &j,
            json!({"Code": "200", "StateUp": 1, "StateDown": 0, "StatePos": 0.4, "value": "0"}),
        );
        assert_eq!(
            v,
            vec![
                ("u-OutputUp".into(), json!(1)),
                ("u-OutputDown".into(), json!(0)),
                ("u-OutputPos".into(), json!(0.4))
            ]
        );
        let s = block("State", &["TQ", "AQ"]);
        assert!(
            values(
                &s,
                json!({"Code": "200", "output": {"name": "Temp", "value": "18.1"}})
            )
            .is_empty()
        );
    }

    #[test]
    fn a_refused_query_says_its_code() {
        assert_eq!(failure(&json!({"Code": "200"})), None);
        assert_eq!(
            failure(&json!({"Code": "500", "value": ""})).as_deref(),
            Some("Code 500")
        );
        assert_eq!(failure(&json!({"code": 404})).as_deref(), Some("Code 404"));
    }
}
