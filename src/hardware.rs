//! Hardware: the extensions and Tree, Air and bus devices of a config.
//!
//! Most changes only touch logic, which the Miniserver loads on a program
//! restart. A change to an extension or a device object (its settings, its
//! serial, adding or removing it) also reconfigures hardware: the Miniserver
//! sends the device a new configuration block, which the device stores in its
//! own flash (Inside-The-Loxone-Miniserver, `NAT/LoxoneLinkNATProtocol.md`,
//! "Configurations in devices"). This module says which objects are devices,
//! what kind they are, which of their attributes reach the device, and what
//! those attributes do there, from the per-device tables of the same source.
//! The tables come from bus captures of older firmware (2019–2021); the
//! attribute names in the `.Loxone` file were matched to them by hand, for the
//! devices listed in [`hint`].
//!
//! It also reads which devices are on the bus now (`lox extensions -o json`,
//! read-only), for the serial check before Apply and the online list after it.

use crate::live::Found;
use crate::log::LoxRead;
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;

/// The Miniserver itself: its settings are not a device's configuration.
const MINISERVER: &str = "LoxLIVE";

/// Whether an object of type `ty` is a device of its own: an extension, a
/// bus interface, a Tree or Air device. Hardware carries its serial in
/// `Serial=` (an Air device its MAC in `IP=`, lox's `loxone-format.md`,
/// "Hardware identity").
pub fn is_device(ty: &str, has_serial: bool) -> bool {
    ty != MINISERVER && (has_serial || ty == "LoxAIRDevice")
}

/// The attribute that identifies a device of type `ty`: another value is
/// another physical device.
pub fn identity(ty: &str) -> &'static str {
    if ty == "LoxAIRDevice" { "IP" } else { "Serial" }
}

/// A serial as hex digits only, upper case, so `b03b368a` and
/// `50:4F:94:…` compare with what the config writes.
pub fn norm_serial(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// What kind of device an object of type `ty` is (with the `TreeType=` of
/// a Tree device), and the device type code the bus uses, when known.
pub fn kind(ty: &str, tree_type: Option<&str>) -> (String, Option<String>) {
    if ty == "TreeDevice" {
        let n = tree_type.and_then(|t| t.trim().parse::<u32>().ok());
        let code = n.map(|n| format!("0x{n:04X}"));
        let name = n
            .and_then(tree_name)
            .map(String::from)
            .unwrap_or_else(|| match &code {
                Some(c) => format!("Tree device {c}"),
                None => "Tree device".to_string(),
            });
        return (name, code);
    }
    let (name, code): (&str, Option<u32>) = match ty {
        "LoxAin" => ("AI Extension", Some(0x19)),
        "LoxAout" => ("AO Extension", Some(0x17)),
        "LoxDigin" => ("DI Extension", Some(0x14)),
        "LoxREL" => ("Relay Extension", Some(0x0B)),
        "LoxDIMM" => ("Dimmer Extension", Some(0x02)),
        "LoxOCEAN" => ("EnOcean Extension", Some(0x03)),
        "LoxMORE" => ("Extension", Some(0x01)),
        "LoxTree" => ("Tree Extension", Some(0x13)),
        "LoxAIR" => ("Air Base Extension", Some(0x0C)),
        "Comm485" => ("RS485 Extension", Some(0x07)),
        "Comm232" => ("RS232 Extension", Some(0x06)),
        "CommDMX" => ("DMX Extension", Some(0x04)),
        "CommIR" => ("IR Extension", Some(0x08)),
        "EIBline" => ("KNX Extension", Some(0x15)),
        "Comm1wire" => ("1-Wire Extension", None),
        "LoxAIRDevice" => ("Air device", None),
        _ => return (ty.to_string(), None),
    };
    (name.to_string(), code.map(|c| format!("0x{c:02X}")))
}

/// Tree device types (`NAT/LoxoneLinkNATProtocol.md`, "Device Type"), with
/// `0x801A` as lox's `tree_kinds` and Loxone Config saves name it (the NAT
/// table, older, lists an unreleased device there).
fn tree_name(n: u32) -> Option<&'static str> {
    Some(match n {
        0x8001 => "Valve Actuator Tree",
        0x8002 => "Motion Sensor Tree",
        0x8003 => "Touch Tree",
        0x8004 => "Universal Tree",
        0x8005 => "Touch Pure Tree",
        0x8006 => "LED Ceiling Light Tree",
        0x8007 => "LED Surface Mount Spot RGBW Tree",
        0x8008 => "LED Spot RGBW Tree Gen 1",
        0x8009 => "NFC Code Touch Tree",
        0x800A => "Weather Station Tree",
        0x800B => "Nano DI Tree",
        0x800C => "RGBW 24V Dimmer Tree",
        0x800D => "Touch Surface Tree",
        0x800E => "LED Surface Mount Spot WW Tree",
        0x800F => "LED Spot WW Tree Gen 1",
        0x8010 => "Room Comfort Sensor Tree",
        0x8011 => "LED Pendulum Slim RGBW Tree",
        0x8012 => "Alarm Siren Tree",
        0x8013 => "Damper Tree",
        0x8014 => "Leaf Tree",
        0x8015 => "Integrated Window Contact Tree",
        0x8016 => "LED Spot RGBW Tree",
        0x8017 => "LED Spot WW Tree",
        0x8018 => "Power Tree",
        0x8019 => "Nano 2 Relay Tree",
        0x801A => "Presence Sensor Tree",
        0x801B => "Magnus Tree",
        0x801C => "NFC Code Touch Tree",
        0x801D => "Tree to Air Bridge",
        0x801E => "Nano Motor Controller Tree",
        0x801F => "Touch Pure Flex Tree",
        0x8020 => "Belimo Tree",
        0x8021 => "Power Supply & Backup",
        0x8022 => "Touch Pure CO2 Tree",
        0x8023 => "Wallbox 11kW 16A Tree",
        _ => return None,
    })
}

const ROOM_COMFORT: u32 = 0x8010;
/// Tree devices with a Touch's audible acknowledgement.
const TOUCHES: &[u32] = &[0x8003, 0x8005, 0x800D, 0x801F, 0x8022];

fn tree_type(t: Option<&str>) -> Option<u32> {
    t.and_then(|t| t.trim().parse().ok())
}

/// What to know about a kind of device beyond its settings.
pub fn note(ty: &str, tree: Option<&str>) -> Option<&'static str> {
    if ty == "TreeDevice" && tree_type(tree) == Some(ROOM_COMFORT) {
        return Some(
            "Its CO2 is an equivalent CO2 (CCS811), not a measured one, and needs several hours of \
             burn-in after power-on: values right after replacing or re-powering it are not reliable.",
        );
    }
    None
}

/// Attributes of a device object that stay in the Miniserver: names,
/// where it is mounted, how Loxone Config draws it. Everything else on a
/// device object counts as its configuration.
const NOT_ON_DEVICE: &[&str] = &[
    "Title",
    "Desc",
    "Installation",
    "SwitchBoard",
    "SwitchBoardRow",
    "SwitchBoardPos",
    "Cl",
    "WF",
    "TId",
];

/// Whether a change of `attr` reconfigures the device: on the device object
/// (`own`) every attribute but those that stay in the Miniserver; on one of
/// its inputs or outputs only those matched to its configuration ([`hint`]).
pub fn reconfigures(dev_ty: &str, tree: Option<&str>, obj_ty: &str, own: bool, attr: &str) -> bool {
    if own {
        !NOT_ON_DEVICE.contains(&attr)
    } else {
        hint(dev_ty, tree, obj_ty, own, attr).is_some()
    }
}

/// What `attr` does on the device, for its tooltip in Properties: on the
/// device object itself (`own`), or on an input or output of a device of
/// type `dev_ty`. Only for attributes matched to the per-device tables.
pub fn hint(
    dev_ty: &str,
    tree: Option<&str>,
    obj_ty: &str,
    own: bool,
    attr: &str,
) -> Option<&'static str> {
    if own {
        return match attr {
            "Serial" => Some(
                "Serial number: the Miniserver finds the device on the bus by it. Another serial is \
                 another physical device (a replacement); until one with this serial answers, the \
                 device's inputs and outputs do not work.",
            ),
            "IP" if dev_ty == "LoxAIRDevice" => Some(
                "The Air device's address, formed from its MAC (its serial): another value is \
                 another physical device.",
            ),
            "TreeType" | "SubType" => Some(
                "Device type: the Miniserver configures only a device of this type at this serial.",
            ),
            "HwVersion" => Some("Hardware version of the device."),
            "AC" if dev_ty == "TreeDevice"
                && tree_type(tree).is_some_and(|t| TOUCHES.contains(&t)) =>
            {
                Some(
                    "Audible acknowledgement: the Touch beeps when a zone is touched. Part of the \
                     configuration the device stores (Touch Tree, on/off).",
                )
            }
            "SSC" if dev_ty == "TreeDevice" && tree_type(tree) == Some(ROOM_COMFORT) => Some(
                "Send interval for temperature and humidity, s (the device stores minutes; default \
                 15 min). It also sends when the temperature changes by more than 1 °C or the \
                 humidity by more than 20 %.",
            ),
            _ => None,
        };
    }
    match (dev_ty, obj_ty, attr) {
        ("LoxAin", "VoltageIn", "MinChange") => Some(
            "Threshold, V (the device stores mV): the AI Extension only sends a new value when the \
             input changed by more than this since the last one it sent.",
        ),
        ("LoxAin", "VoltageIn", "MinTime") => Some(
            "Minimal time between two sends, ms: the AI Extension sends at most one change per \
             this time.",
        ),
        ("LoxAin", "VoltageIn", "AverageTime") => Some(
            "Mean value calculation time, s: the AI Extension samples at 100 Hz and sends the \
             average every this many seconds.",
        ),
        _ => None,
    }
}

/// A device on the bus, as `lox extensions -o json` lists it.
#[derive(Serialize, Debug, PartialEq)]
pub struct OnBus {
    pub name: String,
    /// Hex digits only, upper case ([`norm_serial`]); empty when none.
    pub serial: String,
    pub online: bool,
    #[serde(rename = "type")]
    pub ty: String,
    pub parent: Option<String>,
}

/// `lox extensions -o json`: the extensions and devices the Miniserver
/// knows, with their serial and whether they are online.
pub fn parse(text: &str) -> Result<Vec<OnBus>, String> {
    let v: Value = serde_json::from_str(text.trim())
        .map_err(|e| format!("lox extensions printed no list: {e}"))?;
    let arr = v
        .as_array()
        .ok_or("lox extensions printed no list".to_string())?;
    let s = |d: &Value, k: &str| d.get(k).and_then(Value::as_str).map(String::from);
    Ok(arr
        .iter()
        .map(|d| OnBus {
            name: s(d, "name").unwrap_or_default(),
            serial: norm_serial(&s(d, "serial").unwrap_or_default()),
            online: d.get("online").and_then(Value::as_bool).unwrap_or(false),
            ty: s(d, "type").unwrap_or_default(),
            parent: s(d, "parent"),
        })
        .collect())
}

/// The devices on the bus (`GET /api/hardware`), read with lox's
/// configuration and context, one read at a time.
pub struct Bus {
    read: LoxRead,
}

impl Bus {
    pub fn new(lox: &Path, off: Option<&str>, found: Result<&Found, &String>, start: &Path) -> Bus {
        Bus {
            read: LoxRead::new(lox, off, found, start),
        }
    }

    /// For `/api/history`: whether the bus can be read, and why not.
    pub fn info(&self) -> Value {
        self.read.info()
    }

    /// `{available, devices, fetched}`, read after this request came in; a
    /// failure keeps the devices of the last read that worked (`error`).
    pub async fn fetch(&self) -> Value {
        self.read
            .fetch(
                &["extensions", "-o", "json", "--no-color"],
                "lox extensions",
                |text, _| Ok(json!({ "devices": parse(text)? })),
                json!({ "devices": [] }),
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_are_the_objects_with_a_serial_but_the_miniserver() {
        assert!(is_device("TreeDevice", true));
        assert!(is_device("LoxDigin", true));
        assert!(is_device("LoxAIRDevice", false));
        assert!(!is_device("LoxLIVE", true));
        assert!(!is_device("VoltageIn", false));
        assert_eq!(identity("LoxAIRDevice"), "IP");
        assert_eq!(identity("TreeDevice"), "Serial");
        assert_eq!(norm_serial("50:4f:94:a0"), "504F94A0");
    }

    #[test]
    fn kinds_come_from_the_tree_type_or_the_extension_type() {
        assert_eq!(
            kind("TreeDevice", Some("32771")),
            ("Touch Tree".into(), Some("0x8003".into()))
        );
        assert_eq!(
            kind("TreeDevice", Some("32784")).0,
            "Room Comfort Sensor Tree"
        );
        assert_eq!(kind("TreeDevice", Some("32794")).0, "Presence Sensor Tree");
        assert_eq!(kind("TreeDevice", Some("40000")).0, "Tree device 0x9C40");
        assert_eq!(
            kind("LoxAin", None),
            ("AI Extension".into(), Some("0x19".into()))
        );
        assert_eq!(kind("LoxDigin", None).0, "DI Extension");
        assert!(
            note("TreeDevice", Some("32784"))
                .unwrap()
                .contains("burn-in")
        );
        assert!(note("TreeDevice", Some("32771")).is_none());
    }

    #[test]
    fn only_settings_that_reach_the_device_reconfigure_it() {
        let touch = Some("32771");
        // On the device: everything but names, mounting and drawing.
        assert!(reconfigures("TreeDevice", touch, "TreeDevice", true, "AC"));
        assert!(reconfigures(
            "TreeDevice",
            touch,
            "TreeDevice",
            true,
            "Serial"
        ));
        assert!(!reconfigures(
            "TreeDevice",
            touch,
            "TreeDevice",
            true,
            "Title"
        ));
        assert!(!reconfigures(
            "TreeDevice",
            touch,
            "TreeDevice",
            true,
            "SwitchBoardPos"
        ));
        // On an input: only what the device's table has.
        assert!(reconfigures(
            "LoxAin",
            None,
            "VoltageIn",
            false,
            "MinChange"
        ));
        assert!(!reconfigures("LoxAin", None, "VoltageIn", false, "Title"));
        assert!(!reconfigures(
            "TreeDevice",
            touch,
            "TreeAsensor",
            false,
            "MinChange"
        ));
        // Hints name what the setting does on that device only.
        assert!(
            hint("TreeDevice", touch, "TreeDevice", true, "AC")
                .unwrap()
                .contains("beeps")
        );
        assert!(hint("TreeDevice", Some("32769"), "TreeDevice", true, "AC").is_none());
        assert!(
            hint("TreeDevice", Some("32784"), "TreeDevice", true, "SSC")
                .unwrap()
                .contains("minutes")
        );
        assert!(
            hint("LoxAin", None, "VoltageIn", false, "MinChange")
                .unwrap()
                .contains("mV")
        );
    }

    #[test]
    fn the_bus_list_reads_as_lox_prints_it() {
        // `lox extensions -o json` (lox 0.14), shortened.
        let text = r#"[
          {"name":"Intercom","online":false,"place":"Ogród","serial":"","type":"Intercom","version":""},
          {"devices":13,"errors":0,"name":"Gałąź Tree","online":true,"serial":"13000001","type":"Tree","version":"15.2.9.18"},
          {"name":"WL1","online":true,"parent":"Gałąź Tree","serial":"b03b368a","type":"Tree Device","version":"15.2.9.11"}
        ]"#;
        let l = parse(text).unwrap();
        assert_eq!(l.len(), 3);
        assert_eq!(
            l[2],
            OnBus {
                name: "WL1".into(),
                serial: "B03B368A".into(),
                online: true,
                ty: "Tree Device".into(),
                parent: Some("Gałąź Tree".into()),
            }
        );
        assert!(!l[0].online && l[0].serial.is_empty());
        assert!(parse("Error: no connection").is_err());
        assert!(parse("{}").is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_bus_comes_from_lox_with_vislox_s_context() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let lox = tmp.path().join("lox");
        let f = Found {
            conn: loxone_client::Connection::new("https://192.168.1.120", "u", "p"),
            from: "test".into(),
            ctx: Some("home".into()),
            config_repo: None,
            ms_dir: "192_168_1_120".into(),
            serial: None,
        };
        let script = |body: &str| {
            std::fs::write(&lox, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&lox, std::fs::Permissions::from_mode(0o755)).unwrap();
            Bus::new(&lox, None, Ok(&f), tmp.path())
        };
        let b = script(
            r#"[ "$*" = "--ctx home extensions -o json --no-color" ] || { echo "unexpected: $*" >&2; exit 2; }
echo '[{"name":"WL1","online":true,"serial":"B03B368A","type":"Tree Device"}]'"#,
        );
        let v = b.fetch().await;
        assert_eq!(v["available"], true, "{v}");
        assert_eq!(v["devices"][0]["serial"], "B03B368A");
        assert!(v.get("error").is_none());
        // A failure says why, and has no devices it did not read.
        let b = script("echo 'login refused: Code 4003' >&2; exit 1");
        let v = b.fetch().await;
        assert!(v["error"].as_str().unwrap().contains("Code 4003"), "{v}");
        assert_eq!(v["devices"].as_array().unwrap().len(), 0);
        let off = Bus::new(&lox, Some("turned off (--no-live)"), Ok(&f), tmp.path());
        assert_eq!(off.fetch().await["reason"], "turned off (--no-live)");
    }
}
