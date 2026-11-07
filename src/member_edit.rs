//! Editing the members of a central block (#75): the `members` op of
//! `POST /api/edit`, and the `.members` claims it writes into vislox's file
//! of claims (`<module>/vislox/edits.loxl`).
//!
//! Lox Logic writes a central block's members with one claim (lox's
//! `language.md`, *`.members`*):
//!
//! ```text
//! vx_10000050_0000_0500_ffff504f94000001.members = (vx_licht, vx_10000030_…)
//! ```
//!
//! - The members of a `CentralLight` are `LightController2` blocks, those of
//!   a `CentralShade` `AutoJalousie` blocks: each listed once, never the
//!   central block itself, each an extern or a block of the module.
//! - The list is written in the order given, the order the app lists them in
//!   (`rec=`). `()` empties it.
//! - One claim per central block. Dropping it leaves the members as last
//!   written: lox keeps no lock state for it.
//! - The members of an `Alarm` or `SmokeAlarm` (`Objects=`) stay with Loxone
//!   Config: they name hidden refs wired to the alarm that the list does not
//!   own. They are shown, never edited.
//!
//! Unlike switching times or moods, a member list is no named set that
//! several blocks share: each claim holds its own list, so it is not a
//! [`crate::set_claim::SetKind`]. What it shares with them (how a block of
//! vislox's files is named) comes from [`crate::set_claim`].

use crate::edit::{self, Applied, Ctx, Files, Lock};
use crate::set_claim;
use loxone_config::LoxoneDoc;
use loxone_config::xml::Element;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The most members one list takes (a bound on a request; a house has a
/// few dozen lighting controllers at most).
pub const MAX_MEMBERS: usize = 200;

/// The claims of vislox's file: a central block's slug (an extern of the
/// file, or a block of vislox's files) → its members' slugs, in order.
pub type Members = BTreeMap<String, Vec<String>>;

/// Where a block keeps its member list, and whether vislox edits it:
/// `rec=` on a central block, `Objects=` on an alarm (read-only).
pub fn list_attr(ty: &str) -> Option<(&'static str, bool)> {
    match ty {
        "CentralLight" | "CentralShade" => Some(("rec", true)),
        "Alarm" | "SmokeAlarm" => Some(("Objects", false)),
        _ => None,
    }
}

/// The type a central block's members are: `LightController2` under a
/// `CentralLight`, `AutoJalousie` under a `CentralShade` (lox's rule).
pub fn member_type(ty: &str) -> Option<&'static str> {
    loxone_config::connectors::central_member_type(ty)
}

/// The UUIDs `el` lists as its members (`rec=` or `Objects=`), in order;
/// empty when it lists none or is no such block.
pub fn of(el: &Element) -> Vec<String> {
    let Some((attr, _)) = el.attr("Type").and_then(list_attr) else {
        return Vec::new();
    };
    el.attr_decoded(attr)
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// The comment above the claims in the file.
pub const HEADER: &str = "# Members of central blocks edited in vislox. A claim makes the list the block's whole\n\
                          # member list, in order; deleting it leaves the members as last written.\n";

/// The claims as the file has them (empty when there are none).
pub fn render(m: &Members) -> String {
    let mut out = String::new();
    if m.is_empty() {
        return out;
    }
    out.push('\n');
    out.push_str(HEADER);
    for (c, list) in m {
        out.push_str(&format!("{c}.members = ({})\n", list.join(", ")));
    }
    out
}

/// What vislox's files give the central block `target`: its members' UUIDs
/// in order (a slug lox has not pinned yet stays a slug), or `None` when
/// they claim none.
pub fn current(files: &Files, lock: &Lock, target: &str) -> Option<Vec<String>> {
    files.claims.4.get(target).map(|list| {
        list.iter()
            .map(|s| set_claim::member_of(files, lock, s).unwrap_or_else(|| s.clone()))
            .collect()
    })
}

fn title_of(el: &Element) -> String {
    el.attr_decoded("Title")
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.into_owned())
        .unwrap_or_else(|| el.attr("Type").unwrap_or_default().to_string())
}

/// Apply a `members` op: make `value` the whole member list of the central
/// block `uuid`, in order, or drop vislox's claim (`None`). Its `previous`
/// is the list vislox claimed before (`null`: none), which undoes it.
pub fn apply(
    files: &mut Files,
    ctx: &Ctx,
    uuid: &str,
    value: Option<&Vec<String>>,
) -> Result<Applied, String> {
    let target = set_claim::target_of(files, ctx.lock, uuid);
    let previous = current(files, ctx.lock, &target);
    let done = |p: Option<Vec<String>>| Applied {
        previous: json!(p),
        added: None,
    };
    let Some(el) = edit::find(&ctx.doc.xml.root, uuid) else {
        // A central block the pending config lost (deleted in Loxone Config):
        // only its stale claim can go.
        if value.is_none() && files.claims.4.contains_key(&target) {
            files.claims.4.remove(&target);
            files.claims.prune();
            return Ok(done(previous));
        }
        return Err(format!("no object {uuid} in the pending config"));
    };
    let name = title_of(el);
    let ty = el.attr("Type").unwrap_or_default();
    let want = match (member_type(ty), list_attr(ty)) {
        (Some(w), _) => w,
        (None, Some((_, false))) => {
            return Err(format!(
                "{name}: the members of an alarm (Objects=) stay with Loxone Config: they name \
                 hidden references wired to the alarm, which the list does not own"
            ));
        }
        _ => {
            return Err(format!(
                "{name} is a {ty}: only a CentralLight or a CentralShade has members here"
            ));
        }
    };
    if let Some(s) = ctx.lock.slug_of(uuid)
        && files.block(s).is_none()
    {
        return Err(format!(
            "{name}: the module manages this block; change its members in its .loxl file"
        ));
    }
    let Some(list) = value else {
        // Dropping vislox's claim is always let through: it is how a block
        // the module claims too is mended.
        files.claims.4.remove(&target);
        files.claims.prune();
        return Ok(done(previous));
    };
    if let Some(at) = ctx.members_claimed.get(&uuid.to_ascii_lowercase()) {
        return Err(format!(
            "{name}: {at} claims its members; change them there (a central block takes one \
             members claim)"
        ));
    }
    if list.len() > MAX_MEMBERS {
        return Err(format!(
            "{name}: more than {MAX_MEMBERS} members in one list"
        ));
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut slugs = Vec::with_capacity(list.len());
    let mut externs: Vec<String> = Vec::new();
    for m in list {
        let m = m.trim();
        if m.eq_ignore_ascii_case(uuid) {
            return Err(format!("{name} cannot be a member of itself"));
        }
        let mel = edit::find(&ctx.doc.xml.root, m)
            .ok_or_else(|| format!("{name}: no object {m} in the pending config"))?;
        let mname = title_of(mel);
        let mty = mel.attr("Type").unwrap_or_default();
        if mty != want {
            return Err(format!(
                "{name}: {mname} is a {mty}; the members of a {ty} are {want} blocks"
            ));
        }
        if !seen.insert(m.to_ascii_lowercase()) {
            return Err(format!(
                "{name}: {mname} is listed twice; a block is a member once"
            ));
        }
        let slug = match ctx.lock.slug_of(m) {
            Some(s) if files.block(s).is_some() => s.to_string(),
            Some(_) => {
                return Err(format!(
                    "{name}: {mname} is managed by the module, and vislox's files name only their \
                     own blocks and Loxone Config's; list the members in the module's own file, or \
                     leave {mname} out"
                ));
            }
            None => {
                externs.push(m.to_string());
                edit::slug(m)
            }
        };
        slugs.push(slug);
    }
    for m in externs {
        let c = files.claims.0.entry(m).or_default();
        if c.ty.is_empty() {
            c.ty = want.into();
        }
    }
    // An extern of the file names a central block Loxone Config drew.
    if target == edit::slug(uuid) {
        let c = files.claims.0.entry(uuid.to_string()).or_default();
        c.ty = ty.into();
    }
    files.claims.4.insert(target, slugs);
    files.claims.prune();
    Ok(done(previous))
}

/// After the compile: the central blocks among `uuids` whose member list in
/// `out` is not the one vislox's file claims. lox writes a claim exactly;
/// this catches one it did not write.
pub fn missed(files: &Files, out: &LoxoneDoc, lock: &Lock, uuids: &[String]) -> Vec<String> {
    let mut bad = Vec::new();
    for u in uuids {
        let target = set_claim::target_of(files, lock, u);
        let Some(want) = current(files, lock, &target) else {
            continue;
        };
        let Some(el) = edit::find(&out.xml.root, u) else {
            continue;
        };
        let got: Vec<String> = of(el).iter().map(|x| x.to_ascii_lowercase()).collect();
        let want: Vec<String> = want.iter().map(|x| x.to_ascii_lowercase()).collect();
        if got != want {
            bad.push(title_of(el));
        }
    }
    bad
}

/// Whether the block `uuid` lists other members in `a` than in `b` (or is
/// in only one of them).
pub fn differs(a: &LoxoneDoc, b: &LoxoneDoc, uuid: &str) -> bool {
    let at = |d: &LoxoneDoc| edit::find(&d.xml.root, uuid).map(of);
    at(a) != at(b)
}

/// The blocks a claim names: the central block's slug, then its members'.
pub fn names(m: &Members) -> impl Iterator<Item = (&String, &String)> {
    m.iter()
        .flat_map(|(c, list)| std::iter::once((c, c)).chain(list.iter().map(move |x| (c, x))))
}

/// `GET /api/edits`' `members` (#75): per central block vislox's file
/// claims the members of now or claimed in the deployed revision, its
/// members now (`members`, UUIDs in order; `null`: no claim) and then
/// (`deployed`: the value a Revert sends), whether the pending config lists
/// other members on it than the deployed one (`pending`), and the members
/// the claim names that the config shown does not have (`stale`: deleted in
/// Loxone Config, so lox cannot compile the claim; `missing`: the central
/// block itself is gone).
pub fn edits_section(
    now: &Files,
    was: &Files,
    lock: &Lock,
    pending: Option<&LoxoneDoc>,
    deployed: Option<&LoxoneDoc>,
) -> Value {
    let Some(doc) = pending.or(deployed) else {
        return json!([]);
    };
    let mut blocks: BTreeMap<String, (Option<&str>, Option<&str>)> = BTreeMap::new();
    for (files, now_side) in [(now, true), (was, false)] {
        for t in files.claims.4.keys() {
            if let Some(u) = set_claim::member_of(files, lock, t) {
                let slot = blocks.entry(u).or_default();
                if now_side {
                    slot.0 = Some(t)
                } else {
                    slot.1 = Some(t)
                }
            }
        }
    }
    blocks
        .into_iter()
        .map(|(u, (tn, tw))| {
            let members = tn.and_then(|t| current(now, lock, t));
            let stale: Vec<&String> = members
                .iter()
                .flatten()
                .filter(|m| edit::find(&doc.xml.root, m).is_none())
                .collect();
            json!({
                "uuid": u,
                "members": members,
                "deployed": tw.and_then(|t| current(was, lock, t)),
                "stale": stale,
                "missing": edit::find(&doc.xml.root, &u).is_none(),
                // As for a value: with something pending, a revision that
                // was never deployed differs from it.
                "pending": match (pending, deployed) {
                    (Some(p), Some(d)) => differs(p, d, &u),
                    (Some(_), None) => true,
                    (None, _) => false,
                },
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod fixture {
    use crate::mood_edit::fixture as lights;

    pub use lights::{BAD, FLUR, LICHT};
    /// A CentralLight listing Licht and Flur, in that order.
    pub const ALLE: &str = "10000050-0000-0500-ffff504f94000001";
    /// A CentralShade with no members, and a blind it can take.
    pub const ROLLOS: &str = "10000051-0000-0510-ffff504f94000001";
    pub const ROLLO: &str = "10000052-0000-0520-ffff504f94000001";
    /// An alarm whose members (`Objects=`) stay with Loxone Config.
    pub const ALARM: &str = "10000053-0000-0530-ffff504f94000001";

    /// The house fixture with the three lighting controllers of
    /// [`lights::house`], and the central blocks above on the upper floor.
    pub fn house() -> String {
        let mut s = lights::house();
        let blocks = format!(
            "\t\t\t<C Type=\"CentralLight\" V=\"178\" U=\"{ALLE}\" Title=\"Alle Lichter\" Px=\"5760\" Py=\"7200\" \
             Px2=\"7104\" Py2=\"8160\" Nio=\"0\" rec=\"{LICHT},{FLUR}\" SpStates=\"0\"/>\n\
             \t\t\t<C Type=\"CentralShade\" V=\"178\" U=\"{ROLLOS}\" Title=\"Alle Rollos\" Px=\"5760\" Py=\"9600\" \
             Px2=\"7104\" Py2=\"10560\" Nio=\"0\" SpStates=\"0\"/>\n\
             \t\t\t<C Type=\"AutoJalousie\" V=\"178\" U=\"{ROLLO}\" Title=\"Rollo\" Px=\"5760\" Py=\"12000\" \
             Px2=\"7104\" Py2=\"12960\" Nio=\"0\"/>\n\
             \t\t\t<C Type=\"Alarm\" V=\"178\" U=\"{ALARM}\" Title=\"Alarm\" Px=\"5760\" Py=\"14400\" \
             Px2=\"7104\" Py2=\"15360\" Nio=\"0\" Entries=\"40\" CentralOut=\"1\" Objects=\"{LICHT}\"/>\n"
        );
        let end = "\n\t\t</C>\n\t</C>\n</ControlList>";
        let at = s.rfind(end).expect("end of the upper floor");
        s.insert_str(at + 1, &blocks);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use crate::edit::{Claims, Op};

    fn ctx_parts() -> (LoxoneDoc, Lock) {
        (
            edit::parse_doc(house().as_bytes()).unwrap(),
            Lock::default(),
        )
    }

    macro_rules! ctx {
        ($doc:expr, $lock:expr, $claimed:expr) => {
            Ctx {
                doc: $doc,
                base: $doc,
                lock: $lock,
                deployed: None,
                names: &Default::default(),
                drawn: &Default::default(),
                adoptions: &Default::default(),
                page_adoptions: &Default::default(),
                sched_claimed: &Default::default(),
                moods_claimed: &Default::default(),
                members_claimed: $claimed,
            }
        };
    }

    fn op(uuid: &str, list: Option<&[&str]>) -> Op {
        Op::Members {
            uuid: uuid.into(),
            value: list.map(|l| l.iter().map(|s| s.to_string()).collect()),
        }
    }

    #[test]
    fn a_list_is_claimed_in_order_with_externs_and_undone() {
        let (doc, lock) = ctx_parts();
        let none = BTreeMap::new();
        let ctx = ctx!(&doc, &lock, &none);
        let mut files = Files::default();
        let a = edit::apply_op(&mut files, &ctx, &op(ALLE, Some(&[BAD, LICHT]))).unwrap();
        assert_eq!(a.previous, Value::Null);
        let text = files.claims.render();
        let (c, b, l) = (edit::slug(ALLE), edit::slug(BAD), edit::slug(LICHT));
        assert!(
            text.contains(&format!("extern {c} = CentralLight(uuid: \"{ALLE}\")\n")),
            "{text}"
        );
        assert!(
            text.contains(&format!("extern {b} = LightController2(uuid: \"{BAD}\")\n")),
            "{text}"
        );
        assert!(
            text.contains(&format!("{c}.members = ({b}, {l})\n")),
            "{text}"
        );
        // Read back with lox's parser: the same claims.
        assert_eq!(Claims::parse(&text).unwrap(), files.claims);
        // A new order: previous is the list before.
        let a = edit::apply_op(&mut files, &ctx, &op(ALLE, Some(&[LICHT, BAD]))).unwrap();
        assert_eq!(a.previous, json!([BAD, LICHT]));
        // `()` empties the list; it is still a claim.
        let a = edit::apply_op(&mut files, &ctx, &op(ALLE, Some(&[]))).unwrap();
        assert_eq!(a.previous, json!([LICHT, BAD]));
        assert!(
            files
                .claims
                .render()
                .contains(&format!("{c}.members = ()\n")),
            "{}",
            files.claims.render()
        );
        // The externs of the members went with them.
        assert!(!files.claims.0.contains_key(BAD));
        // Dropped: nothing claimed, nothing left in the file.
        let a = edit::apply_op(&mut files, &ctx, &op(ALLE, None)).unwrap();
        assert_eq!(a.previous, json!([]));
        assert_eq!(files.claims, Claims::default());
        // The undo of a request is built from previous.
        let undo = edit::undo_ops(&[op(ALLE, Some(&[LICHT]))], &[json!([BAD])], &[]).unwrap();
        assert_eq!(undo, vec![op(ALLE, Some(&[BAD]))]);
    }

    #[test]
    fn the_language_rules_are_checked_before_lox_sees_them() {
        let (doc, lock) = ctx_parts();
        let none = BTreeMap::new();
        let ctx = ctx!(&doc, &lock, &none);
        let mut files = Files::default();
        let err = |files: &mut Files, o: Op| edit::apply_op(files, &ctx, &o).unwrap_err();
        let e = err(&mut files, op(ALLE, Some(&[LICHT, LICHT])));
        assert!(e.contains("Licht is listed twice"), "{e}");
        let e = err(&mut files, op(ALLE, Some(&[ROLLO])));
        assert!(
            e.contains(
                "Rollo is a AutoJalousie; the members of a CentralLight are LightController2"
            ),
            "{e}"
        );
        let e = err(&mut files, op(ROLLOS, Some(&[LICHT])));
        assert!(
            e.contains("the members of a CentralShade are AutoJalousie"),
            "{e}"
        );
        let e = err(&mut files, op(ALLE, Some(&[ALLE])));
        assert!(e.contains("cannot be a member of itself"), "{e}");
        let e = err(&mut files, op(ALARM, Some(&[LICHT])));
        assert!(e.contains("stay with Loxone Config"), "{e}");
        let e = err(&mut files, op(LICHT, Some(&[FLUR])));
        assert!(e.contains("only a CentralLight or a CentralShade"), "{e}");
        let e = err(
            &mut files,
            op(ALLE, Some(&["10000099-0000-0990-ffff504f94000001"])),
        );
        assert!(e.contains("no object 10000099"), "{e}");
        assert_eq!(files.claims, Claims::default());
        // A blind under the shade block is fine.
        edit::apply_op(&mut files, &ctx, &op(ROLLOS, Some(&[ROLLO]))).unwrap();
    }

    #[test]
    fn a_block_the_module_manages_or_claims_is_refused() {
        let (doc, mut lock) = ctx_parts();
        lock.slugs.insert("flur".into(), FLUR.into());
        lock.slugs.insert("rollos".into(), ROLLOS.into());
        let claimed = BTreeMap::from([(ALLE.to_string(), "pages/licht.loxl:4".to_string())]);
        let ctx = ctx!(&doc, &lock, &claimed);
        let mut files = Files::default();
        let e = edit::apply_op(&mut files, &ctx, &op(ROLLOS, Some(&[ROLLO]))).unwrap_err();
        assert!(e.contains("the module manages this block"), "{e}");
        let e = edit::apply_op(&mut files, &ctx, &op(ALLE, Some(&[LICHT]))).unwrap_err();
        assert!(e.contains("pages/licht.loxl:4 claims its members"), "{e}");
        let none = BTreeMap::new();
        let ctx = ctx!(&doc, &lock, &none);
        let e = edit::apply_op(&mut files, &ctx, &op(ALLE, Some(&[FLUR]))).unwrap_err();
        assert!(e.contains("Flur is managed by the module"), "{e}");
        // Dropping a claim the module makes too is let through.
        let ctx = ctx!(&doc, &lock, &claimed);
        edit::apply_op(&mut files, &ctx, &op(ALLE, None)).unwrap();
    }

    #[test]
    fn a_stale_claim_is_listed_and_can_be_dropped() {
        let (doc, lock) = ctx_parts();
        let none = BTreeMap::new();
        let ctx = ctx!(&doc, &lock, &none);
        let mut files = Files::default();
        edit::apply_op(&mut files, &ctx, &op(ALLE, Some(&[LICHT, BAD]))).unwrap();
        // Bad deleted in Loxone Config: listed as stale.
        let gone = house().replace(
            &format!("U=\"{BAD}\""),
            "U=\"10000099-0000-0990-ffff504f94000001\"",
        );
        let gone = edit::parse_doc(gone.as_bytes()).unwrap();
        let v = edits_section(&files, &Files::default(), &lock, Some(&gone), None);
        assert_eq!(v[0]["uuid"], ALLE);
        assert_eq!(v[0]["members"], json!([LICHT, BAD]));
        assert_eq!(v[0]["stale"], json!([BAD]));
        assert_eq!(v[0]["pending"], true);
        assert_eq!(v[0]["deployed"], Value::Null);
        // The list without it is written.
        let ctx = ctx!(&gone, &lock, &none);
        edit::apply_op(&mut files, &ctx, &op(ALLE, Some(&[LICHT]))).unwrap();
        assert!(!files.claims.0.contains_key(BAD));
        // The central block itself gone: only its claim can go.
        let lost = house().replace(
            &format!("U=\"{ALLE}\""),
            "U=\"10000098-0000-0980-ffff504f94000001\"",
        );
        let lost = edit::parse_doc(lost.as_bytes()).unwrap();
        let ctx = ctx!(&lost, &lock, &none);
        let e = edit::apply_op(&mut files, &ctx, &op(ALLE, Some(&[LICHT]))).unwrap_err();
        assert!(e.contains("no object"), "{e}");
        edit::apply_op(&mut files, &ctx, &op(ALLE, None)).unwrap();
        assert_eq!(files.claims, Claims::default());
    }

    #[test]
    fn what_lox_wrote_is_checked_and_compared() {
        let (doc, lock) = ctx_parts();
        let none = BTreeMap::new();
        let ctx = ctx!(&doc, &lock, &none);
        let mut files = Files::default();
        edit::apply_op(&mut files, &ctx, &op(ALLE, Some(&[FLUR, LICHT]))).unwrap();
        // The base lists Licht, Flur: not what the claim says.
        assert_eq!(
            missed(&files, &doc, &lock, &[ALLE.into()]),
            ["Alle Lichter"]
        );
        let out = house().replace(
            &format!("rec=\"{LICHT},{FLUR}\""),
            &format!("rec=\"{FLUR},{LICHT}\""),
        );
        let out = edit::parse_doc(out.as_bytes()).unwrap();
        assert!(missed(&files, &out, &lock, &[ALLE.into()]).is_empty());
        assert!(differs(&doc, &out, ALLE));
        assert!(!differs(&doc, &out, ROLLOS));
        // An empty list: no `rec=` at all.
        edit::apply_op(&mut files, &ctx, &op(ALLE, Some(&[]))).unwrap();
        let empty = house().replace(&format!(" rec=\"{LICHT},{FLUR}\""), "");
        let empty = edit::parse_doc(empty.as_bytes()).unwrap();
        assert!(missed(&files, &empty, &lock, &[ALLE.into()]).is_empty());
        // The alarm's members are read too.
        let el = edit::find(&doc.xml.root, ALARM).unwrap();
        assert_eq!(of(el), [LICHT]);
    }

    #[test]
    fn a_block_added_here_is_named_by_its_slug_and_kept_while_listed() {
        let (doc, mut lock) = ctx_parts();
        // Bad stands for a controller added here, as `vx_neu`.
        lock.slugs.insert("vx_neu".into(), BAD.into());
        let page = "# Blocks added in vislox on this page.\npage \"Obergeschoss\"\n\nvx_neu = LightController2(\"Bad\")\n";
        let mut files = Files::from_texts(&BTreeMap::from([(
            "obergeschoss.loxl".to_string(),
            page.to_string(),
        )]))
        .unwrap();
        let none = BTreeMap::new();
        let ctx = ctx!(&doc, &lock, &none);
        edit::apply_op(&mut files, &ctx, &op(ALLE, Some(&[BAD, LICHT]))).unwrap();
        let text = files.claims.render();
        let (c, l) = (edit::slug(ALLE), edit::slug(LICHT));
        assert!(
            text.contains(&format!("{c}.members = (vx_neu, {l})\n")),
            "{text}"
        );
        assert_eq!(
            current(&files, &lock, &c),
            Some(vec![BAD.into(), LICHT.into()])
        );
        let e =
            edit::apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: BAD.into() }).unwrap_err();
        assert!(e.contains(&format!("({c}.members)")), "{e}");
    }

    #[test]
    fn a_claim_on_a_name_vislox_does_not_have_is_refused() {
        let s = edit::slug(ALLE);
        let text = format!(
            "extern {s} = CentralLight(uuid: \"{ALLE}\")\n{s}.members = (vx_gibts_nicht)\n"
        );
        let e = Files::from_texts(&BTreeMap::from([("edits.loxl".to_string(), text)])).unwrap_err();
        assert!(e.contains("vx_gibts_nicht"), "{e}");
        let e = Claims::parse("hand.members = ()\n").unwrap_err();
        assert!(e.contains("vislox did not write"), "{e}");
    }
}
