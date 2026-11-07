//! Editing who gets a push notification (#77): the `recipients` op of
//! `POST /api/edit`, and the `recipients … end` sets and `.recipients` claims
//! it writes into vislox's file of claims (`<module>/vislox/edits.loxl`).
//!
//! Lox Logic writes a notification's recipients with a named set and a claim
//! (lox's `language.md`, *Push notifications*):
//!
//! ```text
//! recipients vx_frost_recipients
//!     user "alice"
//!     group "Familie"
//! end
//!
//! vx_60000010_0000_0100_ffff504f94000001.recipients = vx_frost_recipients
//! ```
//!
//! - The claim makes the set the notification's whole list of recipients
//!   (`ACT=` on it): everyone it leaves out stops getting it, and an empty
//!   set leaves only the members of *Full access*, who get every
//!   notification whatever the set says.
//! - Each user or group once, by a title that names exactly one of its kind;
//!   no schedules. Users and groups stay with Loxone Config and the app, so
//!   an op names them by UUID and vislox writes the title only when it names
//!   exactly that one.
//! - The `Notification` itself is created in Loxone Config (*Messages*): the
//!   claim is on an extern of it, never on a block of a page.
//! - Several notifications may claim one set; one claims one set.
//! - Dropping the claim leaves the recipients as last written (lox keeps no
//!   lock state for it).
//!
//! Everything else is [`crate::access_edit`]'s and [`crate::set_claim`]'s: the
//! same directory of users and groups (by UUID and title, never a
//! credential), the same shared-set rules. The review of who stops getting a
//! notification is #68's (`rights.rs`), read from the compiled config.

use crate::access_edit::Directory;
use crate::edit::{self, Applied, Claims, Ctx, Files, Lock};
use crate::set_claim;
use loxone_config::LoxoneDoc;
use loxone_config::access::PrincipalKind;
use loxone_config::logic::{AccessGrant, Item, RightKind};
use loxone_config::xml::Element;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::blocks::NOTIFICATION;

/// The most recipients one set takes (only bounds a request).
const MAX_RECIPIENTS: usize = 500;

/// One recipient, as an op carries it: a user or group by UUID (titles can
/// change and can repeat; the UUID is what an undo must give back).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Recipient {
    /// `user` or `group`.
    pub kind: String,
    /// The user's or group's UUID.
    pub uuid: String,
}

/// What a `recipients` op gives a notification: as [`crate::access_edit::AccessVal`],
/// with `recipients` for `grants`.
///
/// - `recipients` alone: this list. It goes into the set the notification
///   claims when no other notification claims it, else into a new set.
/// - `set` and `recipients`: write vislox's set `set` (for every
///   notification that claims it) and claim it.
/// - `set` alone: claim vislox's set `set` as it is (*Same recipients as …*).
/// - neither: drop the notification's claim, after `restore`.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct RecipientsVal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipients: Option<Vec<Recipient>>,
    /// Set name → its recipients, written back before the rest.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub restore: BTreeMap<String, Vec<Recipient>>,
}

/// The recipient sets vislox's file writes: its `recipients` sets and claims.
pub type RecipientSets = set_claim::SetStore<RecipientsKind>;

/// Recipients as a kind of set claim ([`set_claim`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecipientsKind;

impl Directory {
    /// A line of a `recipients` set as an op carries it.
    pub fn recipient_of(&self, l: &AccessGrant) -> Result<Recipient, String> {
        let what = l.kind.as_str();
        if l.schedule.is_some() {
            return Err(format!(
                "{what} \"{}\": a recipient takes no schedule",
                l.principal
            ));
        }
        let uuid = Self::uuid_of(what, &l.principal, &self.principals(l.kind))?;
        Ok(Recipient {
            kind: what.to_string(),
            uuid: uuid.to_string(),
        })
    }

    /// An op's recipients as lines, each title naming exactly its object.
    /// That each is in the set once is [`set_claim::SetKind::check`]'s, which
    /// every set written goes through.
    pub fn recipient_lines(&self, list: &[Recipient]) -> Result<Vec<AccessGrant>, String> {
        if list.len() > MAX_RECIPIENTS {
            return Err(format!("at most {MAX_RECIPIENTS} recipients"));
        }
        let mut out = Vec::new();
        for r in list {
            let kind = match r.kind.as_str() {
                "user" => PrincipalKind::User,
                "group" => PrincipalKind::Group,
                k => return Err(format!("`{k}`: a recipient is a user or a group")),
            };
            let principal = Self::title_of(kind.as_str(), &r.uuid, &self.principals(kind))?;
            out.push(AccessGrant {
                kind,
                principal: principal.to_string(),
                schedule: None,
                comment: None,
            });
        }
        Ok(out)
    }
}

impl set_claim::SetKind for RecipientsKind {
    type Line = AccessGrant;
    type Content = Vec<Recipient>;
    type Val = RecipientsVal;
    const KEYWORD: &'static str = "recipients";
    const BLOCK: &'static str = NOTIFICATION;
    const WHAT: &'static str = "recipients";
    const STEM: &'static str = "recipients";
    const HEADER: &'static str = "# Recipients edited in vislox. A claim makes the set everyone who gets the push\n\
         # notification (Full access aside): whoever it leaves out stops getting it. Deleting it leaves the recipients as last written.\n";
    const CONTENT_KEY: &'static str = "recipients";
    const MEMBERS_KEY: &'static str = "notifications";
    const ENTRIES_KEY: &'static str = "recipients";
    const SETS_KEY: &'static str = "recipient_sets";
    type Index<'a> = Directory;

    fn store(c: &Claims) -> &RecipientSets {
        &c.6
    }
    fn store_mut(c: &mut Claims) -> &mut RecipientSets {
        &mut c.6
    }
    fn wrong_type(name: &str) -> String {
        format!(
            "{name}: only a push notification's recipients are written in Lox Logic (the \
             Notification of Loxone Config's Messages, not a block on a page)"
        )
    }
    fn parts(v: &RecipientsVal) -> set_claim::Parts<'_, Self> {
        (v.set.as_ref(), v.recipients.as_ref(), &v.restore)
    }
    fn make(
        set: Option<String>,
        recipients: Option<Vec<Recipient>>,
        restore: BTreeMap<String, Vec<Recipient>>,
    ) -> RecipientsVal {
        RecipientsVal {
            set,
            recipients,
            restore,
        }
    }
    fn index<'a>(doc: &'a LoxoneDoc, _: &'a Lock) -> Directory {
        Directory::read(doc)
    }
    fn content_of(lines: &[AccessGrant], m: &Directory) -> Option<Vec<Recipient>> {
        lines.iter().map(|l| m.recipient_of(l).ok()).collect()
    }
    fn read_item(item: &Item) -> Option<set_claim::SetItem<Self::Line>> {
        let vx = |n: &str| n.starts_with(crate::blocks::SLUG_PREFIX);
        match item {
            Item::Access(a) if a.right == RightKind::Recipients && vx(&a.name) => Some(
                set_claim::SetItem::Set(a.name.clone(), a.grants().cloned().collect()),
            ),
            Item::AccessClaim(c) if c.right == RightKind::Recipients && vx(&c.slug) => {
                Some(set_claim::SetItem::Claim(c.slug.clone(), c.set.clone()))
            }
            _ => None,
        }
    }
    fn op(op: &crate::edit::Op) -> Option<(&str, Option<&Self::Val>)> {
        match op {
            crate::edit::Op::Recipients { uuid, value } => Some((uuid, value.as_ref())),
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
        // Each config's rights read once, not once per notification.
        let (ra, rb) = (crate::rights::read(a), crate::rights::read(b));
        Box::new(move |u| of(&ra, u) != of(&rb, u))
    }
    fn lines_of(
        list: &Vec<Recipient>,
        _: &Ctx,
        dir: &Directory,
        _: &[(String, &Element)],
    ) -> Result<Vec<AccessGrant>, String> {
        dir.recipient_lines(list)
    }
    /// Every title names exactly one user or group in the pending config,
    /// none has a schedule, and each user or group (by UUID) is in the set
    /// once: the one rule for duplicates.
    fn check(
        lines: &[AccessGrant],
        _: &Ctx,
        m: &Directory,
        set: &str,
        _: &[(String, &Element)],
    ) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for l in lines {
            let r = m.recipient_of(l).map_err(|e| format!("`{set}`: {e}"))?;
            if !seen.insert(r.uuid) {
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

/// Apply a `recipients` op: give the notification `uuid` the recipients
/// `value`, or drop vislox's claim on them (`None`). Its `previous` undoes it
/// whole.
pub fn apply(
    files: &mut Files,
    ctx: &Ctx,
    uuid: &str,
    value: Option<&RecipientsVal>,
) -> Result<Applied, String> {
    set_claim::apply::<RecipientsKind>(files, ctx, uuid, value)
}

/// A notification's direct recipients in `rights` (one config's
/// [`crate::rights::read`]): (kind, principal UUID), lowercase. *Full access*
/// is no entry: Loxone Config writes nothing for it.
fn of(
    rights: &BTreeMap<String, crate::rights::Rights>,
    uuid: &str,
) -> Option<BTreeSet<(String, String)>> {
    let r = rights.get(uuid).or_else(|| {
        rights
            .iter()
            .find(|(u, _)| u.eq_ignore_ascii_case(uuid))
            .map(|(_, r)| r)
    })?;
    Some(
        r.list
            .iter()
            .map(|p| (p.kind.to_string(), p.uuid.to_ascii_lowercase()))
            .collect(),
    )
}

/// After the compile: the notifications among `uuids`, those sharing a set
/// with them, and those on the sets `restored`, whose recipients in `out` are
/// not their set's. lox writes a claim exactly; this catches one it did not.
pub fn missed(
    files: &Files,
    out: &LoxoneDoc,
    lock: &Lock,
    uuids: &[String],
    restored: &[String],
) -> Vec<String> {
    let s = &files.claims.6;
    let sets = set_claim::sets_written::<RecipientsKind>(files, lock, uuids, restored);
    let mut bad = Vec::new();
    if sets.is_empty() {
        return bad;
    }
    let rights = crate::rights::read(out);
    let dir = Directory::read(out);
    for set in sets {
        let Some(lines) = s.sets.get(set) else {
            continue;
        };
        let want: Option<BTreeSet<(String, String)>> = {
            use set_claim::SetKind;
            RecipientsKind::content_of(lines, &dir)
                .map(|g| g.into_iter().map(|g| (g.kind, g.uuid)).collect())
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

/// Whether the notification `uuid` has other recipients in `a` than in `b`.
#[cfg(test)]
pub fn differs(a: &LoxoneDoc, b: &LoxoneDoc, uuid: &str) -> bool {
    of(&crate::rights::read(a), uuid) != of(&crate::rights::read(b), uuid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use loxone_config::logic::Module;

    fn r(kind: &str, uuid: &str) -> Recipient {
        Recipient {
            kind: kind.into(),
            uuid: uuid.into(),
        }
    }

    #[test]
    fn recipients_name_their_objects_by_a_unique_title_and_take_no_schedule() {
        use fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let m = Directory::read(&d);
        let lines = m
            .recipient_lines(&[r("user", ALICE), r("group", &FAMILIE.to_ascii_uppercase())])
            .unwrap();
        assert_eq!(
            lines.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ["user \"alice\"", "group \"Familie\""]
        );
        let back: Vec<Recipient> = lines.iter().map(|l| m.recipient_of(l).unwrap()).collect();
        assert_eq!(back, [r("user", ALICE), r("group", FAMILIE)]);
        // Twice in a set is the check's, by UUID (`edit.rs` tests it through the op).
        let e = m.recipient_lines(&[r("user", TWIN1)]).unwrap_err();
        assert!(e.contains("2 users are titled \"chris\""), "{e}");
        let e = m.recipient_lines(&[r("user", FAMILIE)]).unwrap_err();
        assert!(e.contains("no user"), "{e}");
        let e = m.recipient_lines(&[r("robot", ALICE)]).unwrap_err();
        assert!(e.contains("a user or a group"), "{e}");
        // An access line (with a schedule) is no recipient.
        let e = m
            .recipient_of(&AccessGrant {
                kind: PrincipalKind::User,
                principal: "alice".into(),
                schedule: Some("Always".into()),
                comment: None,
            })
            .unwrap_err();
        assert!(e.contains("takes no schedule"), "{e}");
    }

    #[test]
    fn a_set_is_rendered_as_lox_reads_it() {
        use fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let mut s = RecipientSets::default();
        s.sets.insert(
            "vx_frost_recipients".into(),
            Directory::read(&d)
                .recipient_lines(&[r("user", ALICE), r("group", FAMILIE)])
                .unwrap(),
        );
        s.claims.insert("vx_a".into(), "vx_frost_recipients".into());
        let text = s.render();
        assert!(
            text.contains(
                "recipients vx_frost_recipients\n\tuser \"alice\"\n\tgroup \"Familie\"\nend\n\nvx_a.recipients = vx_frost_recipients\n"
            ),
            "{text}"
        );
        let m = Module::parse_fragment(&text).unwrap();
        let item = m
            .items
            .iter()
            .find(|i| matches!(i, Item::Access(_)))
            .unwrap();
        use set_claim::SetKind;
        assert!(matches!(
            RecipientsKind::read_item(item),
            Some(set_claim::SetItem::Set(..))
        ));
        // An access set is not this kind's.
        assert!(crate::access_edit::AccessKind::read_item(item).is_none());
    }

    #[test]
    fn full_access_is_inherited_and_no_entry() {
        use fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let rights = crate::rights::read(&d);
        let frost = &rights[FROST];
        assert_eq!(
            frost
                .list
                .iter()
                .map(|p| p.title.as_str())
                .collect::<Vec<_>>(),
            ["alice"]
        );
        // carol gets it as a member of Full access: in effect, inherited, never listed.
        assert_eq!(
            frost
                .inherited
                .iter()
                .map(|u| u.title.as_str())
                .collect::<Vec<_>>(),
            ["carol"]
        );
        assert_eq!(
            of(&rights, FROST).unwrap(),
            BTreeSet::from([("user".to_string(), ALICE.to_string())])
        );
        assert!(!differs(&d, &d, FROST));
    }
}

/// Test fixtures: the house with users, groups, *Full access* and two push
/// notifications, one of them drawn on a page with its message.
#[cfg(test)]
pub(crate) mod fixture {
    pub const ALICE: &str = "60000001-0000-0010-ffff504f94000001";
    pub const BOB: &str = "60000002-0000-0020-ffff504f94000001";
    pub const CAROL: &str = "60000003-0000-0030-ffff504f94000001";
    pub const FAMILIE: &str = "60000004-0000-0040-ffff504f94000001";
    pub const FULL: &str = "60000005-0000-0050-ffff504f94000001";
    pub const TWIN1: &str = "60000006-0000-0060-ffff504f94000001";
    pub const TWIN2: &str = "60000007-0000-0070-ffff504f94000001";
    pub const FROST: &str = "60000010-0000-0100-ffff504f94000001";
    pub const REGEN: &str = "60000011-0000-0110-ffff504f94000001";
    pub const WARN: &str = "60000020-0000-0200-ffff504f94000001";
    pub const PAGE: &str = "60000030-0000-0300-ffff504f94000001";

    /// The house with users alice (who gets Frost), bob, carol (Full
    /// access), two users titled chris and the group Familie (bob); the
    /// notifications Frost and Regen in *Messages*; and on a page of their
    /// own, Frost drawn as an `OutputRefLM` with its message.
    pub fn house() -> String {
        let p = format!("{}/tests/fixtures/house.Loxone", env!("CARGO_MANIFEST_DIR"));
        let s = std::fs::read_to_string(p).unwrap();
        let users = format!(
            r#"		<C Type="LoxCaption" V="178" U="60000000-0000-0001-ffff504f94000001" Title="Users and Permissions" CaptionType="20" SubType="20">
			<C Type="User" V="178" U="{ALICE}" Title="alice" UsR="129">
				<PH NUM="1">
					<PRM REF="{FROST}" ACT="3"/>
				</PH>
			</C>
			<C Type="User" V="178" U="{BOB}" Title="bob" UsR="129"/>
			<C Type="User" V="178" U="{CAROL}" Title="carol" UsR="-1"/>
			<C Type="User" V="178" U="{TWIN1}" Title="chris" UsR="129"/>
			<C Type="User" V="178" U="{TWIN2}" Title="chris" UsR="129"/>
			<C Type="UserGroup" V="178" U="{FAMILIE}" Title="Familie" Rights="129" User="{BOB}" UserGroups=""/>
			<C Type="UserGroup" V="178" U="{FULL}" Title="Full access" Rights="-1" UgType="4" User="{CAROL}" UserGroups=""/>
		</C>
		<C Type="LoxCaption" V="178" U="60000000-0000-0002-ffff504f94000001" Title="Messages" CaptionType="9" SubType="9">
			<C Type="Notification" V="178" U="{FROST}" Title="Frost" WF="16400" Entries="10"/>
			<C Type="Notification" V="178" U="{REGEN}" Title="Regen" WF="16384" Entries="10"/>
		</C>
		<C Type="Page" V="178" U="{PAGE}" Title="Meldungen" WF="16384">
			<C Type="OutputRefLM" V="178" U="{WARN}" Title="Frost" Px="960" Py="960" Px2="2304" Py2="1656" Cl="0,0,0" Nio="2" Ref="{FROST}">
				<Co K="AI" U="60000020-0000-0201-ffff504f94000001"/>
				<Co K="AQ" U="60000020-0000-0202-ffff504f94000001"/>
				<LoggerMailer RefNotification="{FROST}" On="Frost im Garten" Off="Kein Frost mehr" MinimumTime="43200000" NL="1"/>
			</C>
		</C>
"#
        );
        let anchor = "\t\t<C Type=\"Page\" V=\"178\" U=\"10000002";
        assert!(s.contains(anchor));
        s.replacen(anchor, &format!("{users}{anchor}"), 1)
    }
}
