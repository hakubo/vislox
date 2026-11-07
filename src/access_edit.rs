//! Editing who may open an NFC Code Touch (#76): the `access` op of
//! `POST /api/edit`, and the `access … end` sets and `.access` claims it
//! writes into vislox's file of claims (`<module>/vislox/edits.loxl`).
//!
//! Lox Logic writes a Code Touch's access rights with a named set and a claim
//! (lox's `language.md`, *access*):
//!
//! ```text
//! access vx_haustuer_access
//!     user "alice": "Always"
//!     group "Bewohner": "Workdays"
//! end
//!
//! vx_50000009_0000_0090_ffff504f94000001.access = vx_haustuer_access
//! ```
//!
//! - The claim makes the set the Code Touch's whole list of access rights:
//!   everyone it leaves out is revoked, and an empty set revokes everyone.
//! - Each principal once, each with one access schedule (a `SwitchingTimer`).
//! - Users, groups and schedules stay with Loxone Config and the app. The set
//!   names them by title, and a title that names nothing, or more than one
//!   object of its kind, is a compile error. So an op names them by UUID, and
//!   vislox writes the title only when it names exactly that one object.
//! - `ACT=` (operating the block in the app) is no access right: lox keeps it.
//! - Several Code Touches may claim one set; one Code Touch claims one set.
//! - Dropping the claim leaves the rights as last written (lox keeps no lock
//!   state for it).
//!
//! The review of what a change does to who may open a door is #68's
//! (`rights.rs`): it reads the compiled config, so a change made through this
//! op, a shared set or a removed group shows there and in Apply's confirmation
//! without anything more. Only titles and UUIDs leave this module, never a
//! credential.

use crate::edit::{self, Applied, Claims, Ctx, Files, Lock};
use crate::set_claim;
use loxone_config::LoxoneDoc;
use loxone_config::access::{AccessModel, PrincipalKind};
use loxone_config::logic::{AccessGrant, Item, RightKind};
use loxone_config::xml::Element;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The block type that takes an `access` claim.
pub const CODE_TOUCH: &str = loxone_config::logic::ACCESS_BLOCK_TYPE;

/// The most rights one set takes (only bounds a request).
const MAX_GRANTS: usize = 500;

/// One access right, as an op carries it: a user or group and its access
/// schedule, each by UUID (titles can change and can repeat; the UUID is what
/// an undo must give back).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// `user` or `group`.
    pub kind: String,
    /// The user's or group's UUID.
    pub uuid: String,
    /// The access schedule's UUID.
    pub schedule: String,
}

/// What an `access` op gives a Code Touch.
///
/// - `grants` alone: this list of rights. It goes into the set the Code Touch
///   claims when no other Code Touch claims it, else into a new set.
/// - `set` and `grants`: write the set `set` (a new one, or vislox's set of
///   that name, for every Code Touch that claims it) and claim it.
/// - `set` alone: claim vislox's set `set` as it is (*Same rights as …*).
/// - neither: drop the Code Touch's claim, after `restore`.
///
/// `restore` writes other sets of vislox's back first: what an op's
/// `previous` carries when the op rewrote a set other Code Touches share.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct AccessVal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grants: Option<Vec<Grant>>,
    /// Set name → its rights, written back before the rest.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub restore: BTreeMap<String, Vec<Grant>>,
}

/// The access sets vislox's file writes: its `access` sets and claims.
pub type AccessSets = set_claim::SetStore<AccessKind>;

/// Access rights as a kind of set claim ([`set_claim`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccessKind;

impl set_claim::SetKind for AccessKind {
    type Line = AccessGrant;
    type Content = Vec<Grant>;
    type Val = AccessVal;
    const KEYWORD: &'static str = "access";
    const BLOCK: &'static str = CODE_TOUCH;
    const WHAT: &'static str = "access rights";
    const STEM: &'static str = "access";
    const HEADER: &'static str = "# Access rights edited in vislox. A claim makes the set everyone who may open the\n\
         # Code Touch: whoever it leaves out loses access. Deleting it leaves the rights as last written.\n";
    const CONTENT_KEY: &'static str = "grants";
    const MEMBERS_KEY: &'static str = "code_touches";
    const ENTRIES_KEY: &'static str = "access";
    const SETS_KEY: &'static str = "access_sets";
    type Index<'a> = Directory;

    fn store(c: &Claims) -> &AccessSets {
        &c.5
    }
    fn store_mut(c: &mut Claims) -> &mut AccessSets {
        &mut c.5
    }
    fn wrong_type(name: &str) -> String {
        format!("{name}: only an NFC Code Touch's access rights are written in Lox Logic")
    }
    fn parts(v: &AccessVal) -> set_claim::Parts<'_, Self> {
        (v.set.as_ref(), v.grants.as_ref(), &v.restore)
    }
    fn make(
        set: Option<String>,
        grants: Option<Vec<Grant>>,
        restore: BTreeMap<String, Vec<Grant>>,
    ) -> AccessVal {
        AccessVal {
            set,
            grants,
            restore,
        }
    }
    fn index<'a>(doc: &'a LoxoneDoc, _: &'a Lock) -> Directory {
        Directory::read(doc)
    }
    fn content_of(lines: &[AccessGrant], m: &Directory) -> Option<Vec<Grant>> {
        lines.iter().map(|l| m.grant_of(l).ok()).collect()
    }
    fn read_item(item: &Item) -> Option<set_claim::SetItem<Self::Line>> {
        let vx = |n: &str| n.starts_with(crate::blocks::SLUG_PREFIX);
        match item {
            Item::Access(a) if a.right == RightKind::Access && vx(&a.name) => Some(
                set_claim::SetItem::Set(a.name.clone(), a.grants().cloned().collect()),
            ),
            Item::AccessClaim(c) if c.right == RightKind::Access && vx(&c.slug) => {
                Some(set_claim::SetItem::Claim(c.slug.clone(), c.set.clone()))
            }
            _ => None,
        }
    }
    fn op(op: &crate::edit::Op) -> Option<(&str, Option<&Self::Val>)> {
        match op {
            crate::edit::Op::Access { uuid, value } => Some((uuid, value.as_ref())),
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
        // Each config's rights read once, not once per Code Touch.
        let (ra, rb) = (crate::rights::read(a), crate::rights::read(b));
        Box::new(move |u| of(&ra, u) != of(&rb, u))
    }
    fn lines_of(
        grants: &Vec<Grant>,
        _: &Ctx,
        dir: &Directory,
        _: &[(String, &Element)],
    ) -> Result<Vec<AccessGrant>, String> {
        dir.lines_of(grants)
    }
    /// Every title names exactly one object of its kind in the pending
    /// config, and each principal is in the set once.
    fn check(
        lines: &[AccessGrant],
        _: &Ctx,
        m: &Directory,
        set: &str,
        _: &[(String, &Element)],
    ) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for l in lines {
            m.grant_of(l).map_err(|e| format!("`{set}`: {e}"))?;
            if !seen.insert((l.kind, l.principal.clone())) {
                return Err(format!(
                    "`{set}`: {} \"{}\" is listed twice; each user or group is in a set once",
                    l.kind.as_str(),
                    l.principal
                ));
            }
        }
        Ok(())
    }
}

/// The users, groups and access schedules of a config, by UUID and title:
/// what a set names. No credential is read into it.
pub struct Directory {
    /// (UUID, title) of each user, in document order.
    pub users: Vec<(String, String)>,
    /// (UUID, title, its direct members: users and groups) of each group.
    pub groups: Vec<(String, String, Vec<String>)>,
    /// (UUID, title) of each access schedule.
    pub schedules: Vec<(String, String)>,
    /// The UUIDs of the *Full access* groups (`UgType="4"`): their members
    /// get every push notification whatever a set says (#77).
    pub full_access: Vec<String>,
}

impl Directory {
    pub fn read(doc: &LoxoneDoc) -> Directory {
        let m = AccessModel::read(doc);
        let title = |t: &Option<String>| t.clone().unwrap_or_default();
        let users = m
            .users
            .iter()
            .map(|u| (u.uuid.to_ascii_lowercase(), title(&u.title)))
            .collect();
        let groups = m
            .groups
            .iter()
            .map(|g| {
                let direct = g
                    .members
                    .iter()
                    .chain(&g.subgroups)
                    .map(|u| u.to_ascii_lowercase())
                    .collect();
                (g.uuid.to_ascii_lowercase(), title(&g.title), direct)
            })
            .collect();
        let schedules = m
            .schedules
            .iter()
            .map(|s| (s.uuid.to_ascii_lowercase(), title(&s.title)))
            .collect();
        let full_access = m
            .groups
            .iter()
            .filter(|g| g.kind.as_deref() == Some("4"))
            .map(|g| g.uuid.to_ascii_lowercase())
            .collect();
        Directory {
            users,
            groups,
            schedules,
            full_access,
        }
    }

    pub(crate) fn principals(&self, kind: PrincipalKind) -> Vec<(&str, &str)> {
        match kind {
            PrincipalKind::User => self
                .users
                .iter()
                .map(|(u, t)| (u.as_str(), t.as_str()))
                .collect(),
            PrincipalKind::Group => self
                .groups
                .iter()
                .map(|(u, t, _)| (u.as_str(), t.as_str()))
                .collect(),
        }
    }

    fn schedules(&self) -> Vec<(&str, &str)> {
        self.schedules
            .iter()
            .map(|(u, t)| (u.as_str(), t.as_str()))
            .collect()
    }

    /// The title of `uuid` among `all`, when it names that one object.
    pub(crate) fn title_of<'a>(
        what: &str,
        uuid: &str,
        all: &[(&str, &'a str)],
    ) -> Result<&'a str, String> {
        let u = uuid.to_ascii_lowercase();
        let (_, t) = all
            .iter()
            .find(|(x, _)| *x == u)
            .ok_or_else(|| format!("no {what} {uuid} in the pending config"))?;
        if t.is_empty() {
            return Err(format!(
                "the {what} {uuid} has no title, so Lox Logic cannot name it; give it one in \
                 Loxone Config or the app"
            ));
        }
        if t.chars().any(char::is_control) {
            return Err(format!(
                "the {what} {uuid}'s title is more than one line, which Lox Logic cannot name"
            ));
        }
        let n = all.iter().filter(|(_, x)| x == t).count();
        if n > 1 {
            return Err(format!(
                "{n} {what}s are titled \"{t}\"; give them distinct titles in Loxone Config or the \
                 app first (Lox Logic names them by title)"
            ));
        }
        Ok(t)
    }

    /// The UUID the title `title` names among `all`: exactly one.
    pub(crate) fn uuid_of<'a>(
        what: &str,
        title: &str,
        all: &[(&'a str, &str)],
    ) -> Result<&'a str, String> {
        let hits: Vec<&str> = all
            .iter()
            .filter(|(_, t)| *t == title)
            .map(|(u, _)| *u)
            .collect();
        match hits.as_slice() {
            [one] => Ok(one),
            [] => Err(format!(
                "no {what} titled \"{title}\" in the config (renamed or deleted in Loxone Config?)"
            )),
            many => Err(format!(
                "{} {what}s are titled \"{title}\"; give them distinct titles in Loxone Config or \
                 the app",
                many.len()
            )),
        }
    }

    /// A line of a set as an op carries it.
    pub fn grant_of(&self, l: &AccessGrant) -> Result<Grant, String> {
        let what = l.kind.as_str();
        let uuid = Self::uuid_of(what, &l.principal, &self.principals(l.kind))?;
        let schedule = l
            .schedule
            .as_deref()
            .ok_or_else(|| format!("{what} \"{}\" has no access schedule", l.principal))?;
        let s = Self::uuid_of("access schedule", schedule, &self.schedules())?;
        Ok(Grant {
            kind: what.to_string(),
            uuid: uuid.to_string(),
            schedule: s.to_string(),
        })
    }

    /// An op's rights as lines: each principal once, each title naming
    /// exactly its object.
    pub fn lines_of(&self, grants: &[Grant]) -> Result<Vec<AccessGrant>, String> {
        if grants.len() > MAX_GRANTS {
            return Err(format!("at most {MAX_GRANTS} access rights"));
        }
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for g in grants {
            let kind = match g.kind.as_str() {
                "user" => PrincipalKind::User,
                "group" => PrincipalKind::Group,
                k => return Err(format!("`{k}`: a right is a user's or a group's")),
            };
            let principal = Self::title_of(kind.as_str(), &g.uuid, &self.principals(kind))?;
            if !seen.insert(g.uuid.to_ascii_lowercase()) {
                return Err(format!(
                    "{} \"{principal}\" is listed twice; each user or group is in a set once, \
                     with one access schedule",
                    kind.as_str()
                ));
            }
            let schedule = Self::title_of("access schedule", &g.schedule, &self.schedules())?;
            out.push(AccessGrant {
                kind,
                principal: principal.to_string(),
                schedule: Some(schedule.to_string()),
                comment: None,
            });
        }
        Ok(out)
    }

    /// Group UUID → the users it reaches, through nested groups too, in
    /// document order: one walk up the groups per user.
    pub fn reach(&self) -> BTreeMap<String, Vec<String>> {
        let mut parents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (g, _, direct) in &self.groups {
            for m in direct {
                parents.entry(m.as_str()).or_default().push(g.as_str());
            }
        }
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (u, _) in &self.users {
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            let mut queue = vec![u.as_str()];
            while let Some(x) = queue.pop() {
                for g in parents.get(x).into_iter().flatten() {
                    if seen.insert(g) {
                        queue.push(g);
                    }
                }
            }
            for g in seen {
                out.entry(g.to_string()).or_default().push(u.clone());
            }
        }
        out
    }

    /// The pickers' lists for the browser: users, groups (with the users
    /// they reach, and whether it is *Full access*) and access schedules, by
    /// UUID and title. Personal data: served only to a request with the admin
    /// token. The access editor (#76) and the recipients editor (#77) share it.
    pub fn choices(&self) -> Value {
        let reach = self.reach();
        json!({
            "users": self.users.iter().map(|(u, t)| json!({ "uuid": u, "title": t })).collect::<Vec<_>>(),
            "groups": self.groups.iter().map(|(u, t, _)| json!({ "uuid": u, "title": t, "users": reach.get(u).cloned().unwrap_or_default(), "full_access": self.full_access.contains(u) })).collect::<Vec<_>>(),
            "schedules": self.schedules.iter().map(|(u, t)| json!({ "uuid": u, "title": t })).collect::<Vec<_>>(),
        })
    }
}

/// Apply an `access` op: give the NFC Code Touch `uuid` the rights `value`,
/// or drop vislox's claim on them (`None`). Its `previous` undoes it whole.
pub fn apply(
    files: &mut Files,
    ctx: &Ctx,
    uuid: &str,
    value: Option<&AccessVal>,
) -> Result<Applied, String> {
    set_claim::apply::<AccessKind>(files, ctx, uuid, value)
}

/// A Code Touch's access rights in `rights` (one config's [`crate::rights::read`]):
/// (kind, principal UUID, schedule UUID), lowercase. `ACT=` aside.
fn of(
    rights: &BTreeMap<String, crate::rights::Rights>,
    uuid: &str,
) -> Option<BTreeSet<(String, String, String)>> {
    let r = rights.get(uuid).or_else(|| {
        rights
            .iter()
            .find(|(u, _)| u.eq_ignore_ascii_case(uuid))
            .map(|(_, r)| r)
    })?;
    Some(
        r.list
            .iter()
            .map(|p| {
                (
                    p.kind.to_string(),
                    p.uuid.to_ascii_lowercase(),
                    p.schedule_uuid
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase(),
                )
            })
            .collect(),
    )
}

/// After the compile: the Code Touches among `uuids`, those sharing a set
/// with them, and those on the sets `restored`, whose rights in `out` are not
/// their set's. lox writes a claim exactly; this catches one it did not.
pub fn missed(
    files: &Files,
    out: &LoxoneDoc,
    lock: &Lock,
    uuids: &[String],
    restored: &[String],
) -> Vec<String> {
    let s = &files.claims.5;
    let sets = set_claim::sets_written::<AccessKind>(files, lock, uuids, restored);
    let mut bad = Vec::new();
    if sets.is_empty() {
        return bad;
    }
    // The config's rights and directory, read once.
    let rights = crate::rights::read(out);
    let dir = Directory::read(out);
    for set in sets {
        let Some(lines) = s.sets.get(set) else {
            continue;
        };
        let want: Option<BTreeSet<(String, String, String)>> = {
            use set_claim::SetKind;
            AccessKind::content_of(lines, &dir).map(|g| {
                g.into_iter()
                    .map(|g| (g.kind, g.uuid, g.schedule))
                    .collect()
            })
        };
        for t in s.claimants(set) {
            let Some(u) = set_claim::member_of(files, lock, t) else {
                continue;
            };
            let Some(el) = edit::find(&out.xml.root, &u) else {
                continue;
            };
            if want.is_none() || want != of(&rights, &u) {
                bad.push(el.attr_decoded("Title").unwrap_or_default().into_owned());
            }
        }
    }
    bad
}

/// Whether the Code Touch `uuid` has other access rights in `a` than in `b`
/// (or is in only one of them). For one block; [`set_claim::SetKind::differ`]
/// reads each config once for many.
#[cfg(test)]
pub fn differs(a: &LoxoneDoc, b: &LoxoneDoc, uuid: &str) -> bool {
    of(&crate::rights::read(a), uuid) != of(&crate::rights::read(b), uuid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use loxone_config::logic::Module;

    fn grant(kind: &str, uuid: &str, schedule: &str) -> Grant {
        Grant {
            kind: kind.into(),
            uuid: uuid.into(),
            schedule: schedule.into(),
        }
    }

    #[test]
    fn grants_name_their_objects_by_a_unique_title() {
        use fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let m = Directory::read(&d);
        let lines = m
            .lines_of(&[
                grant("user", ALICE, ALWAYS),
                // Case does not matter: a UUID is a UUID.
                grant("group", &BEWOHNER.to_ascii_uppercase(), WORKDAYS),
            ])
            .unwrap();
        assert_eq!(
            lines.iter().map(ToString::to_string).collect::<Vec<_>>(),
            [
                "user \"alice\": \"Always\"",
                "group \"Bewohner\": \"Workdays\""
            ]
        );
        // And back, by UUID (lowercase).
        let back: Vec<Grant> = lines.iter().map(|l| m.grant_of(l).unwrap()).collect();
        assert_eq!(back[1], grant("group", BEWOHNER, WORKDAYS));
        // Refused before lox sees it.
        let e = m
            .lines_of(&[grant("user", ALICE, ALWAYS), grant("user", ALICE, WORKDAYS)])
            .unwrap_err();
        assert!(e.contains("listed twice"), "{e}");
        let e = m.lines_of(&[grant("user", TWIN1, ALWAYS)]).unwrap_err();
        assert!(e.contains("2 users are titled \"chris\""), "{e}");
        let e = m.lines_of(&[grant("user", BEWOHNER, ALWAYS)]).unwrap_err();
        assert!(e.contains("no user"), "{e}");
        let e = m.lines_of(&[grant("user", ALICE, ALICE)]).unwrap_err();
        assert!(e.contains("no access schedule"), "{e}");
        let e = m.lines_of(&[grant("robot", ALICE, ALWAYS)]).unwrap_err();
        assert!(e.contains("user's or a group's"), "{e}");
        // A group reaches its members, nested groups too.
        assert_eq!(m.reach()[BEWOHNER], [BOB.to_string()]);
        // The pickers' lists carry titles, never a credential.
        let c = serde_json::to_string(&m.choices()).unwrap();
        assert!(c.contains("\"alice\"") && !c.contains("0A1B2C3D4E5F") && !c.contains("00112233"));
    }

    #[test]
    fn a_set_is_rendered_as_lox_reads_it() {
        use fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let mut s = AccessSets::default();
        s.sets.insert(
            "vx_haustuer_access".into(),
            Directory::read(&d)
                .lines_of(&[grant("user", ALICE, ALWAYS)])
                .unwrap(),
        );
        s.claims.insert("vx_a".into(), "vx_haustuer_access".into());
        let text = s.render();
        assert!(
            text.contains(
                "access vx_haustuer_access\n\tuser \"alice\": \"Always\"\nend\n\nvx_a.access = vx_haustuer_access\n"
            ),
            "{text}"
        );
        let m = Module::parse_fragment(&text).unwrap();
        assert_eq!(m.access_decls().count(), 1);
        assert_eq!(m.access_claims().count(), 1);
        // An empty set is a set: it revokes everyone.
        let mut e = AccessSets::default();
        e.sets.insert("vx_none".into(), Vec::new());
        e.claims.insert("vx_a".into(), "vx_none".into());
        let m = Module::parse_fragment(&e.render()).unwrap();
        assert_eq!(m.access_decls().next().unwrap().grants().count(), 0);
    }
}

/// Test fixtures: the house with users, groups, access schedules and two
/// NFC Code Touches.
#[cfg(test)]
pub(crate) mod fixture {
    pub const ALICE: &str = "50000001-0000-0010-ffff504f94000001";
    pub const BOB: &str = "50000002-0000-0020-ffff504f94000001";
    pub const BEWOHNER: &str = "50000003-0000-0030-ffff504f94000001";
    pub const TWIN1: &str = "50000004-0000-0040-ffff504f94000001";
    pub const TWIN2: &str = "50000005-0000-0050-ffff504f94000001";
    pub const ALWAYS: &str = "50000006-0000-0060-ffff504f94000001";
    pub const WORKDAYS: &str = "50000007-0000-0070-ffff504f94000001";
    pub const DOOR: &str = "50000009-0000-0090-ffff504f94000001";
    pub const GARAGE: &str = "5000000b-0000-00b0-ffff504f94000001";

    /// The house with users alice (who may open the door always, and holds
    /// `ACT=` on it: she operates it in the app), bob, two users titled
    /// chris, the group Bewohner (bob), two access schedules, and the Code
    /// Touches Haustür and Garage on a page of their own.
    pub fn house() -> String {
        let p = format!("{}/tests/fixtures/house.Loxone", env!("CARGO_MANIFEST_DIR"));
        let s = std::fs::read_to_string(p).unwrap();
        let users = format!(
            r#"		<C Type="LoxCaption" V="178" U="50000000-0000-0001-ffff504f94000001" Title="Users and Permissions" CaptionType="20" SubType="20">
			<C Type="User" V="178" U="{ALICE}" Title="alice" UsR="-1" NFCArr="0A1B2C3D4E5F" NFCNArr="Schlüssel" CodeArr="00112233">
				<PH NUM="1">
					<PRM REF="{DOOR}" TIM="{ALWAYS}" ACT="3"/>
				</PH>
			</C>
			<C Type="User" V="178" U="{BOB}" Title="bob" UsR="129"/>
			<C Type="User" V="178" U="{TWIN1}" Title="chris" UsR="129"/>
			<C Type="User" V="178" U="{TWIN2}" Title="chris" UsR="129"/>
			<C Type="UserGroup" V="178" U="{BEWOHNER}" Title="Bewohner" Rights="129" User="{BOB}" UserGroups=""/>
			<C Type="SwitchingTimer" V="178" U="{ALWAYS}" Title="Always" M="3" FIX="1"/>
			<C Type="SwitchingTimer" V="178" U="{WORKDAYS}" Title="Workdays" M="3"/>
		</C>
		<C Type="Page" V="178" U="50000008-0000-0080-ffff504f94000001" Title="Zugang" WF="16384">
			<C Type="NfcCodeTouch" V="178" U="{DOOR}" Title="Haustür" Px="960" Py="960" Px2="2304" Py2="1656" Nio="0" WF="16384"/>
			<C Type="NfcCodeTouch" V="178" U="{GARAGE}" Title="Garage" Px="960" Py="2880" Px2="2304" Py2="3576" Nio="0" WF="16384"/>
		</C>
"#
        );
        let anchor = "\t\t<C Type=\"Page\" V=\"178\" U=\"10000002";
        assert!(s.contains(anchor));
        s.replacen(anchor, &format!("{users}{anchor}"), 1)
    }
}
