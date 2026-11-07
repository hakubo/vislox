//! Editing a lighting controller's moods (#74): the `moods` op of
//! `POST /api/edit`, and the `moods … end` sets and `.moods` claims it writes
//! into vislox's file of claims (`<module>/vislox/edits.loxl`).
//!
//! Lox Logic writes a LightController2's moods with a named set and a claim
//! (lox's `language.md`, *moods*):
//!
//! ```text
//! moods vx_licht_moods
//!     "Viel Licht": all on
//!     "Lesen": AQ1 80%, AQ2 temp(60%, 2700K)
//! end
//!
//! vx_21892800_0000_0065_ffff000000000001.moods = vx_licht_moods
//! ```
//!
//! - The claim makes the set the controller's whole mood list, in order, plus
//!   its off mood ("Aus"), which source never lists and which keeps its place.
//! - A mood keeps its identity by name: one the controller has keeps its scene
//!   id (what a button on `Sel<n>` selects); a new name is a new mood with the
//!   next free scene id, and a name the set leaves out is deleted. So a rename
//!   is a delete and an add.
//! - The built-in moods (scene ids 776, 777: "Viel Licht") and any mood the
//!   controller's motion settings name (`<LSConfig OpM= ScMv=>`) stay in the set.
//! - Each circuit takes the values of its kind, read off the controller's
//!   moods: `on`/`off`/`40%` on a switch, `off`/`40%` on a dimmer, `rgb(…)`
//!   on a colour circuit, `temp(…%, …K)` on a tunable-white one. `all <v>` sets
//!   the circuits it fits, the rest stay off.
//! - Several controllers may claim one set; one controller claims one set.
//! - Dropping the claim leaves the moods as the base has them: before Apply,
//!   Loxone Config's; after, the last ones written.
//!
//! An op gives each mood its name and its circuit values in the language's
//! terms (`AQ1 80%, AQ3 on`), parsed with lox's own parser.

use crate::edit::{self, Applied, Claims, Ctx, Files, Lock};
use crate::set_claim;
use loxone_config::LoxoneDoc;
use loxone_config::logic::{Item, Module, MoodLine, MoodSetting};
use loxone_config::moods::{MoodValue, Moods, OFF_SID, encode};
use loxone_config::xml::Element;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The most moods one set takes (Loxone Config's editor offers far fewer;
/// this only bounds a request).
const MAX_MOODS: usize = 64;

/// The built-in moods a set must keep: the alarm-clock mood and "Viel
/// Licht", the motion mood.
const BUILTIN_SIDS: [&str; 2] = ["776", "777"];

/// Why a controller cannot lose a mood (lox refuses a set without it). The
/// one place this rule lives: Properties and the browser get it from here.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Required {
    /// The alarm-clock mood (776) or "Viel Licht" (777).
    Builtin,
    /// The controller's motion settings name it (`<LSConfig OpM= ScMv=>`).
    Motion,
}

impl Required {
    /// The reason, in a sentence: *mood "X" is …*.
    pub fn why(self) -> &'static str {
        match self {
            Required::Builtin => "a built-in mood",
            Required::Motion => "named by the motion settings",
        }
    }
}

/// Why the controller `el` cannot lose mood `m`, if it cannot. Never the off
/// mood, which is never in a set at all.
pub fn required(el: &Element, m: &loxone_config::moods::Mood) -> Option<Required> {
    let sid = m.sid.as_deref().unwrap_or("");
    if m.is_off() {
        None
    } else if BUILTIN_SIDS.contains(&sid) {
        Some(Required::Builtin)
    } else if el
        .child_elements()
        .filter(|c| c.name == "LSConfig")
        .flat_map(|c| ["OpM", "ScMv"].into_iter().filter_map(|a| c.attr(a)))
        .any(|a| a == sid)
    {
        Some(Required::Motion)
    } else {
        None
    }
}

/// One mood, as an op carries it: its name and its circuit values in the
/// language's terms (`all on, AQ2 off`, `AQ1 80%, AQ4 rgb(100, 50, 0)`;
/// empty: every circuit off).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct MoodRow {
    pub name: String,
    #[serde(default)]
    pub values: String,
}

/// What a `moods` op gives a controller.
///
/// - `moods` alone: this mood list. It goes into the set the controller
///   claims when no other controller claims it, else into a new set.
/// - `set` and `moods`: write the set `set` (a new one, or vislox's set of
///   that name, for every controller that claims it) and claim it.
/// - `set` alone: claim vislox's set `set` as it is (*Use the same moods
///   as …*).
/// - neither: drop the controller's claim, after `restore`.
///
/// `restore` writes other sets of vislox's back first: what an op's
/// `previous` carries when the op rewrote a set other controllers share, so
/// that undoing it gives them their moods back too.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct MoodsVal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moods: Option<Vec<MoodRow>>,
    /// Set name → its moods, written back before the rest.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub restore: BTreeMap<String, Vec<MoodRow>>,
}

/// The lighting moods vislox's file writes: its `moods` sets and claims.
pub type MoodSets = set_claim::SetStore<MoodKind>;

/// Lighting moods as a kind of set claim ([`set_claim`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MoodKind;

impl set_claim::SetKind for MoodKind {
    type Line = MoodLine;
    type Content = Vec<MoodRow>;
    type Val = MoodsVal;
    const KEYWORD: &'static str = "moods";
    const BLOCK: &'static str = "LightController2";
    const WHAT: &'static str = "moods";
    const STEM: &'static str = "moods";
    const HEADER: &'static str = "# Lighting moods edited in vislox. A claim makes the set the controller's whole mood\n\
         # list (its off mood aside); deleting it leaves the moods as last written.\n";
    const CONTENT_KEY: &'static str = "moods";
    const MEMBERS_KEY: &'static str = "controllers";
    const ENTRIES_KEY: &'static str = "moods";
    const SETS_KEY: &'static str = "mood_sets";
    type Index<'a> = ();

    fn store(c: &Claims) -> &MoodSets {
        &c.3
    }
    fn store_mut(c: &mut Claims) -> &mut MoodSets {
        &mut c.3
    }
    fn wrong_type(name: &str) -> String {
        format!("{name}: only a lighting controller's moods are written in Lox Logic")
    }
    fn parts(v: &MoodsVal) -> set_claim::Parts<'_, Self> {
        (v.set.as_ref(), v.moods.as_ref(), &v.restore)
    }
    fn make(
        set: Option<String>,
        moods: Option<Vec<MoodRow>>,
        restore: BTreeMap<String, Vec<MoodRow>>,
    ) -> MoodsVal {
        MoodsVal {
            set,
            moods,
            restore,
        }
    }
    fn index<'a>(_: &'a LoxoneDoc, _: &'a Lock) -> Self::Index<'a> {}
    fn content_of(lines: &[MoodLine], _: &()) -> Option<Vec<MoodRow>> {
        Some(rows_of(lines))
    }
    fn read_item(item: &Item) -> Option<set_claim::SetItem<Self::Line>> {
        let vx = |n: &str| n.starts_with(crate::blocks::SLUG_PREFIX);
        match item {
            Item::Moods(m) if vx(&m.name) => Some(set_claim::SetItem::Set(
                m.name.clone(),
                m.moods().cloned().collect(),
            )),
            Item::MoodsClaim(c) if vx(&c.slug) => {
                Some(set_claim::SetItem::Claim(c.slug.clone(), c.moods.clone()))
            }
            _ => None,
        }
    }
    fn op(op: &crate::edit::Op) -> Option<(&str, Option<&Self::Val>)> {
        match op {
            crate::edit::Op::Moods { uuid, value } => Some((uuid, value.as_ref())),
            _ => None,
        }
    }
    fn missed(
        files: &Files,
        out: &LoxoneDoc,
        lock: &Lock,
        uuids: &[String],
        restored: &[String],
    ) -> Vec<String> {
        missed(files, out, lock, uuids, restored)
    }
    fn differ<'a>(a: &'a LoxoneDoc, b: &'a LoxoneDoc) -> Box<dyn Fn(&str) -> bool + 'a> {
        Box::new(move |u| differs(a, b, u))
    }
    fn lines_of(
        rows: &Vec<MoodRow>,
        _: &Ctx,
        _idx: &Self::Index<'_>,
        _: &[(String, &Element)],
    ) -> Result<Vec<MoodLine>, String> {
        lines_of(rows)
    }
    /// Every controller on the set: its off mood never in it, the moods it
    /// cannot lose kept, each circuit a value it takes.
    fn check(
        lines: &[MoodLine],
        _: &Ctx,
        _idx: &Self::Index<'_>,
        set: &str,
        lcs: &[(String, &Element)],
    ) -> Result<(), String> {
        for (_, e) in lcs {
            let c = Controller::read(e)?;
            c.check(lines).map_err(|err| {
                if lcs.len() > 1 {
                    format!("{} (sharing `{set}`): {err}", c.title)
                } else {
                    format!("{}: {err}", c.title)
                }
            })?;
        }
        Ok(())
    }
}

/// A mood's circuit values as text: `AQ1 80%, AQ3 on`.
pub fn values_text(settings: &[MoodSetting]) -> String {
    settings
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The rows of a set, as an op carries them.
pub fn rows_of(lines: &[MoodLine]) -> Vec<MoodRow> {
    lines
        .iter()
        .map(|l| MoodRow {
            name: l.name.clone(),
            values: values_text(&l.settings),
        })
        .collect()
}

/// The lines of a set from an op's rows: each name once and not empty, the
/// values read with lox's parser.
pub fn lines_of(rows: &[MoodRow]) -> Result<Vec<MoodLine>, String> {
    if rows.len() > MAX_MOODS {
        return Err(format!("at most {MAX_MOODS} moods"));
    }
    let mut names = BTreeSet::new();
    let mut out = Vec::new();
    for r in rows {
        let name = r.name.trim();
        if name.is_empty() {
            return Err("a mood needs a name".into());
        }
        if name.chars().any(char::is_control) {
            return Err(format!("mood \"{name}\": a name is one line"));
        }
        if !names.insert(name.to_string()) {
            return Err(format!(
                "two moods are named \"{name}\": a mood is known by its name"
            ));
        }
        let values = r.values.trim();
        let values = if values.is_empty() { "all off" } else { values };
        if values.contains(['\n', '\r', '#']) {
            return Err(format!("mood \"{name}\": the values are one line"));
        }
        let head = MoodLine {
            name: name.to_string(),
            settings: Vec::new(),
            comment: None,
        };
        let text = format!("moods vx_check\n\t{head} {values}\nend\n");
        let parsed = Module::parse_fragment(&text)
            .map_err(|e| format!("mood \"{name}\": `{values}`: {}", short(&e.to_string())))?;
        let line = parsed
            .items
            .into_iter()
            .find_map(|i| match i {
                Item::Moods(m) => m.moods().next().cloned(),
                _ => None,
            })
            .ok_or_else(|| format!("mood \"{name}\": `{values}` is not a mood's values"))?;
        let mut seen = BTreeSet::new();
        for s in &line.settings {
            if !seen.insert(s.circuit) {
                return Err(format!(
                    "mood \"{name}\": {} is given twice",
                    s.circuit.map_or("all".to_string(), |n| format!("AQ{n}"))
                ));
            }
        }
        out.push(MoodLine {
            name: name.to_string(),
            settings: line.settings,
            comment: None,
        });
    }
    Ok(out)
}

/// lox's parse error without its position prefix (`Lox Logic parse error at
/// line 2: `): the line is vislox's wrapper's, not the user's.
fn short(e: &str) -> String {
    e.strip_prefix("Lox Logic parse error at ")
        .and_then(|r| r.split_once(": "))
        .map_or(e, |(_, m)| m)
        .to_string()
}

/// A controller's moods as the editor needs them.
pub struct Controller {
    pub title: String,
    /// Each circuit's value kind (`Q<n> >> 28`), index 0 = AQ1.
    pub prefixes: Vec<u32>,
    /// The names of the moods it cannot lose: built-in, or named by its
    /// motion settings.
    pub required: Vec<(String, &'static str)>,
    /// The off mood's name.
    pub off: Option<String>,
}

impl Controller {
    /// Read a LightController2. A controller without moods yet (minted from
    /// source, never saved by Loxone Config) gets the stock ones on its first
    /// claim: 18 switch circuits, "Viel Licht" kept.
    pub fn read(el: &Element) -> Result<Controller, String> {
        let title = el.attr_decoded("Title").unwrap_or_default().into_owned();
        let moods = Moods::read(el).unwrap_or(Moods {
            outputs: 18,
            moods: vec![
                loxone_config::moods::Mood {
                    name: "Viel Licht".into(),
                    sid: Some("777".into()),
                    cid: None,
                    qs: vec![1; 18],
                },
                loxone_config::moods::Mood {
                    name: "Aus".into(),
                    sid: Some(OFF_SID.into()),
                    cid: None,
                    qs: vec![0; 18],
                },
            ],
        });
        let prefixes = moods.prefixes().map_err(|e| format!("{title}: {e}"))?;
        let required = moods
            .moods
            .iter()
            .filter_map(|m| Some((m.name.clone(), required(el, m)?.why())))
            .collect();
        let off = moods
            .moods
            .iter()
            .find(|m| m.is_off())
            .map(|m| m.name.clone());
        Ok(Controller {
            title,
            prefixes,
            required,
            off,
        })
    }

    /// The circuit values lox writes for mood `m`, or why the circuits
    /// cannot take them: lox's rules (`logic/moods.rs`, `circuit_values`).
    pub fn values(&self, m: &MoodLine) -> Result<Vec<u32>, String> {
        let all = m
            .settings
            .iter()
            .find(|s| s.circuit.is_none())
            .map_or(MoodValue::Off, |s| s.value);
        if let Some(s) = m.settings.iter().find(|s| {
            s.circuit
                .is_some_and(|n| usize::from(n) > self.prefixes.len() || n == 0)
        }) {
            return Err(format!(
                "mood \"{}\", `{s}`: the controller has {} circuits (AQ1 … AQ{})",
                m.name,
                self.prefixes.len(),
                self.prefixes.len()
            ));
        }
        self.prefixes
            .iter()
            .enumerate()
            .map(
                |(i, &prefix)| match m.settings.iter().find(|s| s.circuit == Some(i as u8 + 1)) {
                    Some(s) => encode(prefix, s.value)
                        .map_err(|e| format!("mood \"{}\": AQ{} is {e}", m.name, i + 1)),
                    None => Ok(encode(prefix, all).unwrap_or(prefix << 28)),
                },
            )
            .collect()
    }

    /// Check a set for this controller, as lox would compile it.
    pub fn check(&self, lines: &[MoodLine]) -> Result<(), String> {
        if let Some(off) = &self.off
            && lines.iter().any(|l| &l.name == off)
        {
            return Err(format!(
                "\"{off}\" is the controller's off mood, which is always there and never in the set"
            ));
        }
        for (name, why) in &self.required {
            if !lines.iter().any(|l| &l.name == name) {
                return Err(format!(
                    "mood \"{name}\" is {why} and must stay in the set (with any values); it cannot \
                     be deleted or renamed"
                ));
            }
        }
        for l in lines {
            self.values(l)?;
        }
        Ok(())
    }

    /// The moods the set writes: name and circuit values, in order.
    pub fn expected(&self, lines: &[MoodLine]) -> Option<Vec<(String, Vec<u32>)>> {
        lines
            .iter()
            .map(|l| Some((l.name.clone(), self.values(l).ok()?)))
            .collect()
    }
}

/// Apply a `moods` op: give the LightController2 `uuid` the moods `value`,
/// or drop vislox's claim on them (`None`). Its `previous` undoes it whole:
/// the controller's claim as it was, and the moods of every other
/// controller whose set the op rewrote (`restore`).
pub fn apply(
    files: &mut Files,
    ctx: &Ctx,
    uuid: &str,
    value: Option<&MoodsVal>,
) -> Result<Applied, String> {
    set_claim::apply::<MoodKind>(files, ctx, uuid, value)
}

/// A controller's moods as the compiled config has them, but the off mood:
/// name and circuit values, in order.
fn compiled_of(el: &Element) -> Option<Vec<(String, Vec<u32>)>> {
    let m = Moods::read(el)?;
    Some(
        m.moods
            .into_iter()
            .filter(|x| !x.is_off())
            .map(|x| (x.name, x.qs))
            .collect(),
    )
}

/// After the compile: the controllers among `uuids`, those sharing a set with
/// them, and those on the sets `restored`, whose moods in `out` are not
/// their set's. lox writes a claim exactly; this catches one it did not write.
pub fn missed(
    files: &Files,
    out: &LoxoneDoc,
    lock: &Lock,
    uuids: &[String],
    restored: &[String],
) -> Vec<String> {
    let s = &files.claims.3;
    let sets = set_claim::sets_written::<MoodKind>(files, lock, uuids, restored);
    let mut bad = Vec::new();
    for set in sets {
        let Some(lines) = s.sets.get(set) else {
            continue;
        };
        for t in s.claimants(set) {
            let Some(u) = set_claim::member_of(files, lock, t) else {
                continue;
            };
            let Some(el) = edit::find(&out.xml.root, &u) else {
                continue;
            };
            let title = el.attr_decoded("Title").unwrap_or_default().into_owned();
            let want = Controller::read(el).ok().and_then(|c| c.expected(lines));
            if want.is_none() || want != compiled_of(el) {
                bad.push(title);
            }
        }
    }
    bad
}

/// Whether the controller `uuid` has other moods in `a` than in `b` (or is
/// in only one of them).
pub fn differs(a: &LoxoneDoc, b: &LoxoneDoc, uuid: &str) -> bool {
    let of = |d: &LoxoneDoc| edit::find(&d.xml.root, uuid).and_then(compiled_of);
    of(a) != of(b)
}

/// The circuit a `Circuit<n>:` setting of a lighting controller declares,
/// checked: `"switch"` or `"dimmer"`, only on a circuit the controller's
/// moods do not hold colours on (RGB and tunable-white circuits stay with
/// Loxone Config).
pub fn circuit_setting(el: &Element, setting: &str, value: &str) -> Result<(), String> {
    let n = loxone_config::connectors::circuit_param("LightController2", setting)
        .ok_or_else(|| format!("{setting} is not a circuit lox declares (Circuit1 … Circuit5)"))?;
    if !loxone_config::connectors::CIRCUIT_KINDS
        .iter()
        .any(|(k, _)| *k == value)
    {
        return Err(format!("{setting}: \"switch\" or \"dimmer\""));
    }
    if let Some(m) = Moods::read(el)
        && let Ok(p) = m.prefixes()
        && p.get(n - 1).is_some_and(|p| *p >= 4)
    {
        return Err(format!(
            "{setting}: AQ{n} is a colour circuit; RGB and tunable-white circuits stay with Loxone \
             Config"
        ));
    }
    Ok(())
}

/// Whether the moods set `set` (`lines`), which vislox claims for the
/// controller `el`, still fits circuit `AQ<n>` once `Circuit<n>:` is `value`
/// (`None`: no declaration, so the kind `base` has, if it has the
/// controller). The claim writes the circuit's values anew on every compile,
/// so a value the new kind does not take would make lox refuse the compile.
pub fn circuit_against_set(
    el: &Element,
    base: Option<&Element>,
    n: usize,
    value: Option<&str>,
    set: &str,
    lines: &[MoodLine],
) -> Result<(), String> {
    let mut c = Controller::read(el)?;
    let prefix = match value {
        Some(v) => loxone_config::connectors::CIRCUIT_KINDS
            .iter()
            .find(|(k, _)| *k == v)
            .map(|(_, p)| *p),
        None => base
            .and_then(|b| Controller::read(b).ok())
            .and_then(|b| b.prefixes.get(n - 1).copied()),
    };
    let (Some(prefix), Some(slot)) = (prefix, c.prefixes.get_mut(n - 1)) else {
        return Ok(());
    };
    *slot = prefix;
    let kind = if prefix == 1 { "dimmer" } else { "switch" };
    c.check(lines).map_err(|e| {
        format!(
            "the moods vislox claims (`{set}`) do not fit AQ{n} as a {kind}: {e}. Change those \
             values in Edit moods first (a {kind} takes {}), or Revert the moods",
            if prefix == 1 {
                "off or a percentage"
            } else {
                "on, off or a percentage"
            }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, values: &str) -> MoodRow {
        MoodRow {
            name: name.into(),
            values: values.into(),
        }
    }

    #[test]
    fn rows_read_with_loxs_parser_and_back() {
        let lines = lines_of(&[
            row("Viel Licht", "all on"),
            row("Lesen", "AQ1 80%, AQ2 temp(60%, 2700K)"),
            row("Kino", "AQ4 rgb(0, 0, 20)"),
            row("Dunkel", ""),
        ])
        .unwrap();
        assert_eq!(
            lines.iter().map(ToString::to_string).collect::<Vec<_>>(),
            [
                "\"Viel Licht\": all on",
                "\"Lesen\": AQ1 80%, AQ2 temp(60%, 2700K)",
                "\"Kino\": AQ4 rgb(0, 0, 20)",
                "\"Dunkel\": all off",
            ]
        );
        assert_eq!(rows_of(&lines)[1].values, "AQ1 80%, AQ2 temp(60%, 2700K)");
        // Refused before lox sees it.
        assert!(lines_of(&[row("", "all on")]).is_err());
        assert!(
            lines_of(&[row("A", "all on"), row("A", "")])
                .unwrap_err()
                .contains("two moods")
        );
        // lox's message, with the mood it is about and without the wrapper's line.
        let e = lines_of(&[row("A", "AQ1 bright")]).unwrap_err();
        assert!(
            e.starts_with("mood \"A\": `AQ1 bright`: expected a value. A mood is `\"<name>\": all"),
            "{e}"
        );
        assert!(!e.contains("line 2"), "{e}");
        assert!(lines_of(&[row("A", "AQ1 on # x")]).is_err());
        assert!(
            lines_of(&[row("A", "AQ1 on, AQ1 off")])
                .unwrap_err()
                .contains("twice")
        );
        // A quote in a name is written as lox reads it.
        let q = lines_of(&[row("Say \"hi\"", "all on")]).unwrap();
        assert_eq!(q[0].name, "Say \"hi\"");
    }

    #[test]
    fn a_set_is_rendered_as_lox_writes_it() {
        let mut s = MoodSets::default();
        s.sets.insert(
            "vx_licht_moods".into(),
            lines_of(&[row("Viel Licht", "all on"), row("Lesen", "AQ1 80%")]).unwrap(),
        );
        s.claims.insert("vx_a".into(), "vx_licht_moods".into());
        let text = s.render();
        assert!(
            text.contains(
                "moods vx_licht_moods\n\t\"Viel Licht\": all on\n\t\"Lesen\": AQ1 80%\nend\n\nvx_a.moods = vx_licht_moods\n"
            ),
            "{text}"
        );
        // lox reads it back.
        let m = Module::parse_fragment(&text).unwrap();
        assert_eq!(m.mood_sets().count(), 1);
        assert_eq!(m.moods_claims().count(), 1);
    }
}

/// Test fixtures: lighting controllers on the house fixture's upper floor.
#[cfg(test)]
pub(crate) mod fixture {
    pub const LICHT: &str = "10000030-0000-0300-ffff504f94000001";
    pub const FLUR: &str = "10000031-0000-0310-ffff504f94000001";
    pub const BAD: &str = "10000032-0000-0320-ffff504f94000001";

    /// Tunable white at `b`% and `k` Kelvin on a circuit of prefix 6.
    pub fn tw(b: u32, k: u32) -> u32 {
        (6 << 28) + 200_000_000 + 10_000 * b + k
    }

    /// A LightController2: AQ1 a switch (named *Deckenlicht*), AQ2 a dimmer
    /// (no actuator), AQ3 tunable white, AQ4 … AQ18 switches; the moods
    /// "Viel Licht" (777), "Lesen" (1), "Kino" (2) and "Aus" (778, last).
    /// `Sel1` is wired (a button selects "Lesen") when `button`. With `dimmer1`,
    /// AQ1 is a dimmer instead.
    pub fn controller(u: &str, title: &str, y: u32, button: bool, dimmer1: bool) -> String {
        let p1: u32 = if dimmer1 { 1 << 28 } else { 0 };
        let mood = |name: &str, sid: &str, cid: u32, flag: &str, q: [u32; 3], rest: u32| {
            let mut s = format!(
                "<LightsceneC Name=\"{name}\" UUID=\"{}{cid:02}\"{flag} Outputs=\"18\" SID=\"{sid}\" CID=\"{cid}\"",
                &u[..33]
            );
            for (i, v) in q.iter().chain(std::iter::repeat_n(&rest, 15)).enumerate() {
                s.push_str(&format!(" Q{}=\"{v}\"", i + 1));
            }
            s + "/>"
        };
        let co = |k: &str, n: u32, wired: Option<&str>| {
            let id = format!("{}{n:01x}-00ff504f94000001", &u[..17]);
            match wired {
                Some(from) => format!("<Co K=\"{k}\" U=\"{id}\"><In Input=\"{from}\"/></Co>"),
                None => format!("<Co K=\"{k}\" U=\"{id}\"/>"),
            }
        };
        format!(
            "\t\t\t<C Type=\"LightController2\" V=\"178\" U=\"{u}\" Title=\"{title}\" Px=\"960\" Py=\"{y}\" \
             Px2=\"3648\" Py2=\"{}\" Nio=\"5\" NameAI0=\"Deckenlicht\" PresM0=\"65535\">{}{}{}{}{}\
             <LightscenesC FC=\"0040FFFF\" Outputs=\"18\" Num=\"4\">{}{}{}{}</LightscenesC>\
             <LSConfig OpM=\"778\" ScMv=\"777\" ScAc=\"1\"/></C>\n",
            y + 2232,
            co(
                "Sel1",
                1,
                button.then_some("10000004-0000-0043-02ff504f94000001")
            ),
            co("Sel2", 2, None),
            co("AQ1", 3, None),
            co("AQ2", 4, None),
            co("AQ3", 5, None),
            mood(
                "Viel Licht",
                "777",
                22,
                " Move=\"true\"",
                [
                    p1 + if dimmer1 { 100 } else { 1 },
                    (1 << 28) + 100,
                    tw(100, 2700)
                ],
                1
            ),
            mood(
                "Lesen",
                "1",
                12,
                "",
                [p1 + if dimmer1 { 80 } else { 1 }, (1 << 28) + 80, 6 << 28],
                0
            ),
            mood("Kino", "2", 13, "", [p1, 1 << 28, tw(20, 2700)], 0),
            mood(
                "Aus",
                "778",
                11,
                " ReadOnly=\"true\"",
                [p1, 1 << 28, 6 << 28],
                0
            ),
        )
    }

    /// The house fixture with three controllers on the upper floor: Licht
    /// (a button selects "Lesen"), Flur (the same circuits) and Bad (AQ1 a
    /// dimmer).
    pub fn house() -> String {
        let p = format!("{}/tests/fixtures/house.Loxone", env!("CARGO_MANIFEST_DIR"));
        let mut s = std::fs::read_to_string(p).unwrap();
        let all = controller(LICHT, "Licht", 7200, true, false)
            + &controller(FLUR, "Flur", 9600, false, false)
            + &controller(BAD, "Bad", 12000, false, true);
        let end = "\n\t\t</C>\n\t</C>\n</ControlList>";
        let at = s.rfind(end).expect("end of the upper floor");
        s.insert_str(at + 1, &all);
        s
    }
}
