//! The tiles of a Touch Pure Flex (#78), as a `.Loxone` file stores them, and
//! the blocks linked to its API connector.
//!
//! A Touch Pure Flex is a `TreeDevice` with a `<btns>` child (lox's
//! `loxone-format.md`, *Touch Pure Flex tiles*): one `<btn i="…">` per tile,
//! each with three functions, `f1` (the click), `f2` (`bT="1"`, the arrow up)
//! and `f3` (`bT="2"`, the arrow down). A function carries `api=` (the
//! command, `SET(As;Co;pulse)`), `cT=` (the status text) and `ctrls=` (the
//! blocks the command addresses, comma-separated UUIDs). The layout (which
//! tiles exist, where, how large, their icons) is Loxone Config's.
//!
//! A command reaches the blocks *API-linked* to the device: a block whose
//! `OutputAPI` is wired to the device's API connector (`ApiActor.I`), directly
//! or through an `OutputRef` mirroring the actor whose `AQ` feeds its `I`.
//! API wires are recorded in reverse, so either direction counts. This is
//! lox's own reading (`logic/tiles.rs`, `ApiLinks`), which lox does not export.

use loxone_config::LoxoneDoc;
use loxone_config::doc::ports;
use loxone_config::xml::Element;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The three functions of a tile, in element order: the key Lox Logic names
/// it by, and its element.
pub const FUNCTIONS: [(&str, &str); 3] = [("click", "f1"), ("up", "f2"), ("down", "f3")];

/// The block type a Touch Pure Flex is (with a `<btns>` layout).
pub const DEVICE: &str = "TreeDevice";

/// Command abbreviations whose block type lox knows: a target of another type
/// is refused (lox's `KNOWN`).
const KNOWN: &[(&str, &[&str])] = &[("As", &["AutoJalousie"]), ("Lico", &["LightController2"])];

/// The block types `api` addresses, when lox knows every abbreviation in it;
/// `None` when any is unknown, or there is none (lox's `addressed_types`).
pub fn addressed_types(api: &str) -> Option<Vec<&'static str>> {
    let abbrs = loxone_config::logic::abbreviations(api);
    if abbrs.is_empty() {
        return None;
    }
    let mut types = Vec::new();
    for a in abbrs {
        types.extend(KNOWN.iter().find(|(k, _)| *k == a)?.1);
    }
    Some(types)
}

/// One function of a tile as the config stores it.
#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Function {
    /// The command (`api=`); `None`: the function does nothing.
    pub api: Option<String>,
    /// The status text (`cT=`).
    pub text: Option<String>,
    /// The blocks the command addresses (`ctrls=`), in order, as stored.
    pub ctrls: Vec<String>,
}

impl Function {
    /// `ctrls` as compared: lowercase (a save may write them either way).
    pub fn ctrls_lower(&self) -> Vec<String> {
        self.ctrls.iter().map(|u| u.to_ascii_lowercase()).collect()
    }

    fn same(&self, o: &Function) -> bool {
        self.api == o.api && self.text == o.text && self.ctrls_lower() == o.ctrls_lower()
    }
}

/// One tile: its index and its three functions (click, up, down).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    pub index: u32,
    pub functions: [Function; 3],
}

impl Tile {
    /// Whether its functions are the same as `o`'s (`ctrls` case aside).
    pub fn same(&self, o: &Tile) -> bool {
        self.index == o.index
            && self
                .functions
                .iter()
                .zip(&o.functions)
                .all(|(a, b)| a.same(b))
    }
}

/// Whether `el` is a Touch Pure Flex: a `TreeDevice` with a tile layout.
pub fn is_flex(el: &Element) -> bool {
    el.attr("Type") == Some(DEVICE) && el.child_elements().any(|c| c.name == "btns")
}

/// The tiles of a Touch Pure Flex, in the layout's order; `None` for any
/// other object.
pub fn tiles_of(el: &Element) -> Option<Vec<Tile>> {
    if el.attr("Type") != Some(DEVICE) {
        return None;
    }
    let btns = el.child_elements().find(|c| c.name == "btns")?;
    let text = |f: &Element, a: &str| {
        f.attr_decoded(a)
            .map(|v| v.into_owned())
            .filter(|v| !v.is_empty())
    };
    Some(
        btns.child_elements()
            .filter(|b| b.name == "btn")
            .filter_map(|b| {
                let index = b.attr("i")?.trim().parse().ok()?;
                let functions = FUNCTIONS.map(|(_, element)| {
                    b.child_elements()
                        .find(|f| f.name == element)
                        .map(|f| Function {
                            api: text(f, "api"),
                            text: text(f, "cT"),
                            ctrls: text(f, "ctrls")
                                .map(|c| {
                                    c.split(',')
                                        .map(str::trim)
                                        .filter(|u| !u.is_empty())
                                        .map(String::from)
                                        .collect()
                                })
                                .unwrap_or_default(),
                        })
                        .unwrap_or_default()
                });
                Some(Tile { index, functions })
            })
            .collect(),
    )
}

/// The indices of a Touch Pure Flex's tiles, in the layout's order.
pub fn indices(el: &Element) -> Vec<u32> {
    tiles_of(el)
        .unwrap_or_default()
        .iter()
        .map(|t| t.index)
        .collect()
}

/// Whether the device `uuid` has other tile programs in `a` than in `b` (or
/// is a Touch Pure Flex in only one of them).
pub fn differ(a: &LoxoneDoc, b: &LoxoneDoc, uuid: &str) -> bool {
    let at = |d: &LoxoneDoc| find_ci(&d.xml.root, uuid).and_then(tiles_of);
    match (at(a), at(b)) {
        (Some(x), Some(y)) => x.len() != y.len() || x.iter().zip(&y).any(|(p, q)| !p.same(q)),
        (None, None) => false,
        _ => true,
    }
}

/// The object with UUID `uuid`, whatever the case of either.
pub fn find_ci<'a>(el: &'a Element, uuid: &str) -> Option<&'a Element> {
    if el.attr("U").is_some_and(|u| u.eq_ignore_ascii_case(uuid)) {
        return Some(el);
    }
    el.child_elements().find_map(|c| find_ci(c, uuid))
}

/// The blocks API-linked to each Touch Pure Flex of a config: device UUID
/// (lowercase) → the linked blocks' UUIDs (as the config writes them) and
/// types, in UUID order.
#[derive(Default, Debug, Clone)]
pub struct Links(pub BTreeMap<String, Vec<(String, String)>>);

impl Links {
    /// Read `doc` once: every Touch Pure Flex and what is linked to it.
    pub fn read(doc: &LoxoneDoc) -> Links {
        let objects = doc.objects();
        let el = |i: usize| doc.element_at(&objects[i].path);
        // The API connectors of the Touch Pure Flex devices: actor UUID →
        // device UUID (lowercase).
        let mut actors: BTreeMap<&str, String> = BTreeMap::new();
        for o in &objects {
            let Some(e) = doc.element_at(&o.path).filter(|e| is_flex(e)) else {
                continue;
            };
            for c in e.child_elements() {
                if c.name == "C"
                    && c.attr("Type") == Some("ApiActor")
                    && let Some(u) = c.attr("U")
                {
                    actors.insert(u, o.uuid.to_ascii_lowercase());
                }
            }
        }
        let mut out = Links::default();
        if actors.is_empty() {
            return out;
        }
        // Port UUID → (owner object, port key).
        let mut port_owner: BTreeMap<String, (usize, String)> = BTreeMap::new();
        for (i, _) in objects.iter().enumerate() {
            if let Some(e) = el(i) {
                for p in ports(e) {
                    port_owner.insert(p.uuid, (i, p.key));
                }
            }
        }
        let wires: Vec<(String, String)> = doc
            .wires()
            .into_iter()
            .map(|w| (w.from_port, w.to_port))
            .collect();
        let owner = |port: &str| port_owner.get(port).map(|(i, k)| (*i, k.as_str()));
        // OutputRefs mirroring an actor whose `AQ` feeds its `I`: object →
        // the device.
        let mut mirrors: BTreeMap<usize, &String> = BTreeMap::new();
        for (from, to) in &wires {
            let (Some((a, "I")), Some((r, "AQ"))) = (owner(to), owner(from)) else {
                continue;
            };
            let au = objects[a].uuid.as_str();
            if objects[r].block_type == "OutputRef"
                && let Some(dev) = actors.get(au)
                && el(r).and_then(|e| e.attr("Ref")) == Some(au)
            {
                mirrors.insert(r, dev);
            }
        }
        // The device a port lets a command in at: an actor's `I`, or a
        // mirror's `AI`.
        let entry = |port: &str| -> Option<&String> {
            let (i, key) = owner(port)?;
            match key {
                "I" => actors.get(objects[i].uuid.as_str()),
                "AI" => mirrors.get(&i).copied(),
                _ => None,
            }
        };
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        for (from, to) in &wires {
            for (e, other) in [(from, to), (to, from)] {
                let Some((b, "OutputAPI")) = owner(other) else {
                    continue;
                };
                let Some(dev) = entry(e) else { continue };
                let o = &objects[b];
                if seen.insert((dev.clone(), o.uuid.to_ascii_lowercase())) {
                    out.0
                        .entry(dev.clone())
                        .or_default()
                        .push((o.uuid.clone(), o.block_type.clone()));
                }
            }
        }
        for v in out.0.values_mut() {
            v.sort_by_key(|(u, _)| u.to_ascii_lowercase());
        }
        out
    }

    /// The blocks linked to the device `uuid`: (UUID, type).
    pub fn of(&self, uuid: &str) -> &[(String, String)] {
        self.0
            .get(&uuid.to_ascii_lowercase())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// The type of `block` when it is linked to the device `uuid`.
    pub fn linked(&self, uuid: &str, block: &str) -> Option<&str> {
        self.of(uuid)
            .iter()
            .find(|(u, _)| u.eq_ignore_ascii_case(block))
            .map(|(_, t)| t.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixture::*;

    #[test]
    fn tiles_are_read_by_index_with_their_functions() {
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let flex = find_ci(&d.xml.root, FLEX).unwrap();
        assert!(is_flex(flex));
        let t = tiles_of(flex).unwrap();
        assert_eq!(t.iter().map(|t| t.index).collect::<Vec<_>>(), [2, 3, 5]);
        assert_eq!(t[0].functions, <[Function; 3]>::default());
        assert_eq!(t[1].functions[0].api.as_deref(), Some("SET(As;Tg;pulse)"));
        assert_eq!(t[1].functions[0].text.as_deref(), Some("=ECHO(As)"));
        assert_eq!(t[1].functions[0].ctrls, [NORD]);
        assert_eq!(t[1].functions[1].ctrls, [NORD, SUED]);
        // The `&amp;` of a chained command is read decoded.
        assert_eq!(
            t[2].functions[0].api.as_deref(),
            Some("SET(Lico;M+;pulse)&WAIT(500)")
        );
        // The Touch Tree of the same config has no tile layout.
        let touch = find_ci(&d.xml.root, TOUCH).unwrap();
        assert!(!is_flex(touch));
        assert!(tiles_of(touch).is_none());
        assert!(!differ(&d, &d, FLEX));
    }

    #[test]
    fn linked_blocks_are_found_directly_and_through_a_mirror() {
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let l = Links::read(&d);
        let of: Vec<(&str, &str)> = l
            .of(&FLEX.to_ascii_uppercase())
            .iter()
            .map(|(u, t)| (u.as_str(), t.as_str()))
            .collect();
        // Nord directly (its reverse wire), Süd and the light through the
        // OutputRef; Ost is wired to nothing.
        assert_eq!(
            of,
            [
                (NORD, "AutoJalousie"),
                (SUED, "AutoJalousie"),
                (LIGHT, "LightController2")
            ]
        );
        assert_eq!(
            l.linked(FLEX, &SUED.to_ascii_uppercase()),
            Some("AutoJalousie")
        );
        assert_eq!(l.linked(FLEX, OST), None);
        assert!(l.of(TOUCH).is_empty());
    }

    #[test]
    fn commands_address_the_types_lox_knows() {
        assert_eq!(
            addressed_types("SET(As;Co;pulse)"),
            Some(vec!["AutoJalousie"])
        );
        assert_eq!(
            addressed_types("SET(Lico;M+;pulse)&WAIT(500)"),
            Some(vec!["LightController2"])
        );
        assert_eq!(addressed_types("SET(As;Co)&SET(Xy;Tg)"), None);
        assert_eq!(addressed_types("WAIT(500)"), None);
    }
}

/// Test fixtures: the hardware config with a Touch Pure Flex on the Tree
/// Extension, three shades and a lighting controller.
#[cfg(test)]
pub(crate) mod fixture {
    pub const FLEX: &str = "20000007-0000-0070-ffff504f94000002";
    pub const ACTOR: &str = "20000007-0000-0071-ffff504f94000002";
    pub const TOUCH: &str = "20000003-0000-0030-ffff504f94000002";
    pub const NORD: &str = "50000010-0000-0100-ffff504f94000002";
    pub const SUED: &str = "50000011-0000-0110-ffff504f94000002";
    pub const OST: &str = "50000012-0000-0120-ffff504f94000002";
    pub const LIGHT: &str = "50000013-0000-0130-ffff504f94000002";
    pub const MIRROR: &str = "50000014-0000-0140-ffff504f94000002";
    pub const PAGE: &str = "50000000-0000-0000-ffff504f94000002";

    /// The connector UUID `n` of block `u`.
    pub fn co(u: &str, n: u32) -> String {
        format!("{}{n:x}-00ff504f94000002", &u[..17])
    }

    /// `hardware.Loxone` with a Touch Pure Flex "Flex Küche" (tiles 2, 3
    /// and 5; tile 3 moves Nord on a click and Nord and Süd on the arrow up,
    /// tile 5 sets a mood) on the Tree Extension, and a page "Beschattung"
    /// with the shades Nord (its `OutputAPI` wired to the API connector,
    /// recorded in reverse), Süd and the light Licht (through an `OutputRef`
    /// of the actor), and Ost, linked to nothing.
    pub fn house() -> String {
        let p = format!(
            "{}/tests/fixtures/hardware.Loxone",
            env!("CARGO_MANIFEST_DIR")
        );
        let s = std::fs::read_to_string(p).unwrap();
        let ai = co(ACTOR, 1);
        let flex = format!(
            r#"					<C Type="TreeDevice" TId="TpfTree" V="178" U="{FLEX}" Title="Flex Küche" WF="16384" Serial="B0500001" SubType="32799" TreeType="32799" cfgId="1" Artn="100481">
						<C Type="ApiActor" V="178" U="{ACTOR}" Title="API Connector">
							<Co K="I" U="{ai}"/>
						</C>
						<btns>
							<btn i="2" x="6" y="7" h="13" w="15">
								<f1></f1>
								<f2 bT="1"/>
								<f3 bT="2"/>
							</btn>
							<btn ci="1" i="3" x="37" y="7" h="13" w="15">
								<f1 api="SET(As;Tg;pulse)" cT="=ECHO(As)" ctrls="{NORD}"/>
								<f2 api="SET(As;Co;pulse)" cT="=ECHO(As)" bT="1" ctrls="{NORD},{SUED}"/>
								<f3 bT="2"/>
							</btn>
							<btn ci="2" i="5" x="6" y="37" h="13" w="15">
								<f1 api="SET(Lico;M+;pulse)&amp;WAIT(500)"/>
								<f2 bT="1"/>
								<f3 bT="2"/>
							</btn>
						</btns>
					</C>
"#
        );
        let jal = |u: &str, title: &str, y: u32, link: Option<&str>| {
            let api = match link {
                Some(to) => format!(
                    "\t\t\t\t<Co K=\"OutputAPI\" U=\"{}\">\n\t\t\t\t\t<In Input=\"{to}\"/>\n\t\t\t\t</Co>\n",
                    co(u, 1)
                ),
                None => format!("\t\t\t\t<Co K=\"OutputAPI\" U=\"{}\"/>\n", co(u, 1)),
            };
            format!(
                "\t\t\t<C Type=\"AutoJalousie\" V=\"178\" U=\"{u}\" Title=\"{title}\" Px=\"960\" Py=\"{y}\" Px2=\"2304\" Py2=\"{}\" Nio=\"1\" WF=\"16384\">\n{api}\t\t\t</C>\n",
                y + 696
            )
        };
        let mirror_ai = co(MIRROR, 1);
        let page = format!(
            "\t\t<C Type=\"Page\" V=\"178\" U=\"{PAGE}\" Title=\"Beschattung\" WF=\"16384\">\n{}{}{}\
             \t\t\t<C Type=\"LightController2\" V=\"178\" U=\"{LIGHT}\" Title=\"Licht\" Px=\"4800\" Py=\"960\" Px2=\"6144\" Py2=\"1656\" Nio=\"1\" WF=\"16384\">\n\
             \t\t\t\t<Co K=\"OutputAPI\" U=\"{}\">\n\t\t\t\t\t<In Input=\"{mirror_ai}\"/>\n\t\t\t\t</Co>\n\t\t\t</C>\n\
             \t\t\t<C Type=\"OutputRef\" V=\"178\" U=\"{MIRROR}\" Title=\"API Connector\" Px=\"9600\" Py=\"960\" Px2=\"10944\" Py2=\"1656\" Nio=\"2\" Ref=\"{ACTOR}\">\n\
             \t\t\t\t<Co K=\"AI\" U=\"{mirror_ai}\">\n\t\t\t\t\t<In Input=\"{}\"/>\n\t\t\t\t\t<In Input=\"{}\"/>\n\t\t\t\t</Co>\n\
             \t\t\t\t<Co K=\"AQ\" U=\"{}\"/>\n\t\t\t</C>\n\t\t</C>\n",
            jal(NORD, "Nord", 960, Some(&ai)),
            jal(SUED, "Süd", 2880, None),
            jal(OST, "Ost", 4800, None),
            co(LIGHT, 1),
            co(SUED, 1),
            co(LIGHT, 1),
            co(MIRROR, 2),
        );
        // The mirror's distribution into the actor: `ApiActor.I ← OutputRef.AQ`.
        let flex = flex.replace(
            &format!("<Co K=\"I\" U=\"{ai}\"/>"),
            &format!(
                "<Co K=\"I\" U=\"{ai}\">\n\t\t\t\t\t\t\t\t<In Input=\"{}\"/>\n\t\t\t\t\t\t\t</Co>",
                co(MIRROR, 2)
            ),
        );
        let anchor = "\t\t\t\t\t<C Type=\"TreeDevice\" TId=\"RoomsensorTree\"";
        assert!(s.contains(anchor));
        let s = s.replacen(anchor, &format!("{flex}{anchor}"), 1);
        let end = "\t</C>\n</ControlList>";
        assert!(s.contains(end), "the document's end");
        s.replacen(end, &format!("{page}{end}"), 1)
    }
}
