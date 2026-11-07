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

use crate::edit::{self, Applied, Ctx, Files, Lock};
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

/// The mood sets and claims of vislox's file of claims.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct MoodSets {
    /// Set name → its moods, as lox reads them.
    pub sets: BTreeMap<String, Vec<MoodLine>>,
    /// The slug of a controller (an extern of the file, or a block of
    /// vislox's files) → the set it claims.
    pub claims: BTreeMap<String, String>,
}

impl MoodSets {
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty() && self.claims.is_empty()
    }

    /// The controllers that claim `set`.
    pub fn claimants(&self, set: &str) -> Vec<&str> {
        self.claims
            .iter()
            .filter(|(_, s)| s.as_str() == set)
            .map(|(t, _)| t.as_str())
            .collect()
    }

    /// Drop the sets no controller claims.
    pub fn prune(&mut self) {
        let used: BTreeSet<String> = self.claims.values().cloned().collect();
        self.sets.retain(|s, _| used.contains(s));
    }

    /// The sets, then the claims, as the file has them.
    pub fn render(&self) -> String {
        let mut out = String::new();
        if self.is_empty() {
            return out;
        }
        out.push_str(
            "\n# Lighting moods edited in vislox. A claim makes the set the controller's whole mood\n\
             # list (its off mood aside); deleting it leaves the moods as last written.\n",
        );
        for (name, lines) in &self.sets {
            out.push_str(&format!("moods {name}\n"));
            for l in lines {
                out.push_str(&format!("\t{l}\n"));
            }
            out.push_str("end\n");
        }
        out.push('\n');
        for (t, s) in &self.claims {
            out.push_str(&format!("{t}.moods = {s}\n"));
        }
        out
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

/// lox's parse error without its position prefix.
fn short(e: &str) -> String {
    e.rsplit_once(": ").map_or(e, |(_, m)| m).to_string()
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
        let motion: BTreeSet<String> = el
            .child_elements()
            .filter(|c| c.name == "LSConfig")
            .flat_map(|c| ["OpM", "ScMv"].into_iter().filter_map(|a| c.attr(a)))
            .map(String::from)
            .collect();
        let mut required = Vec::new();
        for m in moods.moods.iter().filter(|m| !m.is_off()) {
            let sid = m.sid.as_deref().unwrap_or("");
            if BUILTIN_SIDS.contains(&sid) {
                required.push((m.name.clone(), "a built-in mood"));
            } else if motion.contains(sid) {
                required.push((m.name.clone(), "named by the motion settings"));
            }
        }
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

/// What `files` give the controller `target`: its set and the set's moods.
pub fn current(files: &Files, target: &str) -> Option<MoodsVal> {
    let s = &files.claims.3;
    let set = s.claims.get(target)?;
    Some(MoodsVal {
        set: Some(set.clone()),
        moods: s.sets.get(set).map(|l| rows_of(l)),
        restore: BTreeMap::new(),
    })
}

/// Write `rows` into vislox's set `set`, checked for every controller that
/// claims it (and `also`, about to claim it).
fn write_set(
    files: &mut Files,
    ctx: &Ctx,
    set: &str,
    rows: Option<&[MoodRow]>,
    also: Option<(&str, &Element)>,
) -> Result<(), String> {
    let mut lcs: Vec<&Element> = also.map(|(_, e)| e).into_iter().collect();
    for t in files.claims.3.claimants(set) {
        if also.is_some_and(|(a, _)| a == t) {
            continue;
        }
        if let Some(u) = crate::sched_edit::timer_of(files, ctx.lock, t)
            && let Some(e) = edit::find(&ctx.doc.xml.root, &u)
        {
            lcs.push(e);
        }
    }
    if let Some(rows) = rows {
        let lines = lines_of(rows)?;
        files.claims.3.sets.insert(set.to_string(), lines);
    }
    let lines = files
        .claims
        .3
        .sets
        .get(set)
        .ok_or_else(|| format!("vislox has no mood set `{set}`; give the moods"))?;
    let shared = lcs.len() > 1;
    for e in lcs {
        let c = Controller::read(e)?;
        c.check(lines).map_err(|err| {
            if shared {
                format!("{} (sharing `{set}`): {err}", c.title)
            } else {
                format!("{}: {err}", c.title)
            }
        })?;
    }
    Ok(())
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
    let el = edit::find(&ctx.doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let name = el.attr_decoded("Title").unwrap_or_default().into_owned();
    if el.attr("Type") != Some("LightController2") {
        return Err(format!(
            "{name}: only a lighting controller's moods are written in Lox Logic"
        ));
    }
    let target = match ctx.lock.slug_of(uuid) {
        Some(s) if files.block(s).is_some() => s.to_string(),
        Some(_) => {
            return Err(format!(
                "{name}: the module manages this block; change its moods in its .loxl file"
            ));
        }
        None => edit::slug(uuid),
    };
    let before = files.claims.3.clone();
    let mut previous = current(files, &target);
    if let Some(v) = value {
        for (set, rows) in &v.restore {
            if !files.claims.3.sets.contains_key(set) {
                return Err(format!(
                    "{name}: vislox's mood set `{set}` is gone; its moods cannot be put back"
                ));
            }
            write_set(files, ctx, set, Some(rows), None)?;
        }
    }
    match value {
        Some(v) if v.set.is_some() || v.moods.is_some() => {
            // A controller takes one mood list. Dropping vislox's claim is
            // always let through: it is how one the module claims too is mended.
            if let Some(at) = ctx.moods_claimed.get(&uuid.to_ascii_lowercase()) {
                return Err(format!(
                    "{name}: {at} claims its moods; change them there (a controller takes one mood \
                     list)"
                ));
            }
            let s = &files.claims.3;
            let set = match &v.set {
                Some(n) => n.trim().to_string(),
                None => match s.claims.get(&target) {
                    Some(own) if s.claimants(own) == [target.as_str()] => own.clone(),
                    _ => crate::adopt::default_slug(
                        &format!("{name} moods"),
                        "LightController2",
                        &|x| crate::sched_edit::taken(files, ctx, x),
                    ),
                },
            };
            if !s.sets.contains_key(&set) {
                crate::adopt::check_slug(&set).map_err(|e| format!("The set's name: {e}"))?;
                if crate::sched_edit::taken(files, ctx, &set) {
                    return Err(format!(
                        "The set's name: `{set}` is taken in the module; pick another"
                    ));
                }
                if v.moods.is_none() {
                    return Err(format!(
                        "{name}: vislox has no mood set `{set}`; give the moods"
                    ));
                }
            }
            write_set(files, ctx, &set, v.moods.as_deref(), Some((&target, el))).map_err(|e| {
                if e.starts_with(&name) {
                    e
                } else {
                    format!("{name}: {e}")
                }
            })?;
            files.claims.3.claims.insert(target.clone(), set.clone());
            if target == edit::slug(uuid) {
                let c = files.claims.0.entry(uuid.to_string()).or_default();
                c.ty = "LightController2".into();
            }
        }
        _ => {
            files.claims.3.claims.remove(&target);
        }
    }
    files.claims.3.prune();
    files.claims.prune();
    // The sets the op rewrote that other controllers still claim: their old
    // moods go into the undo, unless the undo's own set has them.
    let mut restore = BTreeMap::new();
    for (set, lines) in &before.sets {
        let now = files.claims.3.sets.get(set);
        let others = files.claims.3.claimants(set).iter().any(|t| *t != target);
        let own = previous.as_ref().and_then(|p| p.set.as_ref()) == Some(set);
        if now.is_some_and(|n| n != lines) && others && !own {
            restore.insert(set.clone(), rows_of(lines));
        }
    }
    if !restore.is_empty() {
        previous.get_or_insert_with(MoodsVal::default).restore = restore;
    }
    Ok(Applied {
        previous: serde_json::json!(previous),
        added: None,
    })
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
    let mut sets: BTreeSet<&str> = restored.iter().map(String::as_str).collect();
    for u in uuids {
        let target = lock
            .slug_of(u)
            .filter(|t| files.block(t).is_some())
            .map(String::from)
            .unwrap_or_else(|| edit::slug(u));
        if let Some(set) = s.claims.get(&target) {
            sets.insert(set);
        }
    }
    let mut bad = Vec::new();
    for set in sets {
        let Some(lines) = s.sets.get(set) else {
            continue;
        };
        for t in s.claimants(set) {
            let Some(u) = crate::sched_edit::timer_of(files, lock, t) else {
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
        assert!(lines_of(&[row("A", "AQ1 bright")]).is_err());
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
