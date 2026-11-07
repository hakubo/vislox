//! Who may open an NFC Code Touch, and who gets a push notification (#68).
//!
//! Neither is on a logic page. Loxone Config keeps both on the users and
//! groups of the *Users and Permissions* tree (lox's `loxone-format.md`,
//! *Users, groups and access rights*), read with lox's own
//! [`AccessModel`]:
//!
//! - a `<PRM REF="<block>" TIM="<schedule>"/>` in a user's or group's
//!   `<PH>` list lets it open the NFC Code Touch `REF=` names, at the
//!   times of the access schedule (`SwitchingTimer`) `TIM=` names;
//! - a `<PRM REF="<notification>" ACT="…"/>` makes it a recipient of the
//!   push notification (`Notification`) `REF=` names. Members of the
//!   *Full access* group (`UgType="4"`) always get it, as lox says.
//!
//! A Lox Logic `access` or `recipients` claim owns these rights on its block,
//! so the compiled config is where its effect shows: whoever the set leaves
//! out loses the right. This module reads them per block from a config, and
//! compares two configs block by block, for the review and Apply's
//! confirmation. A group's right reaches its members (and the members of the
//! groups it lists), so the comparison also says which users gain or lose the
//! right in effect, not only which entries changed.
//!
//! Only titles leave this module: no credential, no other attribute of a
//! user. Titles are still personal: they are served with the rest of the
//! review, never logged.

use loxone_config::LoxoneDoc;
use loxone_config::access::AccessModel;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The right on an NFC Code Touch: who may open it, and when.
pub const ACCESS: &str = "access";
/// The right on a push notification: who gets it.
pub const RECIPIENTS: &str = "recipients";
/// A Daytimer's switching times, for [`Change`]s of the review that come from
/// the schedule comparison (`model.rs`), not from here.
pub const SCHEDULE: &str = "schedule";

/// The block types the rights are on, as lox names them.
const CODE_TOUCH: &str = "NfcCodeTouch";
const NOTIFICATION: &str = "Notification";
/// `UgType=` of the *Full access* group.
const FULL_ACCESS: &str = "4";

/// A user or group that holds a right on a block.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    /// `user` or `group`.
    pub kind: &'static str,
    pub uuid: String,
    pub title: String,
    /// The access schedule's title (access only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    /// The access schedule's UUID: what identifies the right (two schedules
    /// may share a title).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_uuid: Option<String>,
    /// A group's direct members, by title: its users, then its groups.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
}

/// A user, by UUID and title.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Named {
    pub title: String,
    pub uuid: String,
}

/// One block's right in one config.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Rights {
    /// [`ACCESS`] or [`RECIPIENTS`].
    pub kind: &'static str,
    /// Who holds it directly: users first, then groups, each in document
    /// order. An access right is one entry per user or group and schedule:
    /// a principal with two `<PRM>`s on one block (two schedules) is there
    /// twice.
    pub list: Vec<Principal>,
    /// The users who hold it in effect: directly, through a group (nested
    /// groups too), or, for a notification, as members of *Full access*.
    /// By title.
    pub users: Vec<Named>,
    /// For a notification: the users of [`Rights::users`] who get it only as
    /// members of *Full access* (#77). Loxone Config shows them as inherited
    /// recipients that cannot be unticked, and a set cannot take them away.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inherited: Vec<Named>,
    /// For an access right: each of those users (by UUID) → the access
    /// schedules (UUIDs) that let them in, directly or through a group. A
    /// user who keeps access under fewer of them loses opening times
    /// (#76). Not served: the review says it per user.
    #[serde(skip)]
    pub schedules: BTreeMap<String, BTreeSet<String>>,
}

/// The rights of every NFC Code Touch and push notification in `doc`, by the
/// block's UUID. A block nobody holds a right on is there too, with empty
/// lists.
pub fn read(doc: &LoxoneDoc) -> BTreeMap<String, Rights> {
    let types: BTreeMap<String, String> = doc
        .objects()
        .into_iter()
        .filter(|o| o.block_type == CODE_TOUCH || o.block_type == NOTIFICATION)
        .map(|o| (o.uuid, o.block_type))
        .collect();
    if types.is_empty() {
        return BTreeMap::new();
    }
    let m = AccessModel::read(doc);
    let title = |uuid: &str, t: &Option<String>| t.clone().unwrap_or_else(|| uuid.to_string());
    let schedule_title = |uuid: &str| {
        m.schedules
            .iter()
            .find(|s| s.uuid == uuid)
            .and_then(|s| s.title.clone())
            .unwrap_or_else(|| uuid.to_string())
    };
    let member_titles = |g: &loxone_config::access::Group| -> Vec<String> {
        let users = g.members.iter().map(|u| match m.user(u) {
            Some(x) => title(u, &x.title),
            None => u.clone(),
        });
        let groups = g.subgroups.iter().map(|u| match m.group(u) {
            Some(x) => title(u, &x.title),
            None => u.clone(),
        });
        users.chain(groups).collect()
    };
    let mut out: BTreeMap<String, Rights> = types
        .iter()
        .map(|(u, ty)| {
            let kind = if ty == CODE_TOUCH { ACCESS } else { RECIPIENTS };
            (
                u.clone(),
                Rights {
                    kind,
                    list: Vec::new(),
                    users: Vec::new(),
                    inherited: Vec::new(),
                    schedules: BTreeMap::new(),
                },
            )
        })
        .collect();
    let holders = m
        .users
        .iter()
        .map(|u| ("user", &u.uuid, &u.title, &u.permissions, Vec::new()))
        .chain(
            m.groups
                .iter()
                .map(|g| ("group", &g.uuid, &g.title, &g.permissions, member_titles(g))),
        );
    for (kind, uuid, t, perms, members) in holders {
        for p in perms {
            let Some(r) = out.get_mut(&p.target) else {
                continue;
            };
            // A Code Touch's right is its `TIM=`; a notification's, its `ACT=`.
            let schedule_uuid = match r.kind {
                ACCESS => match &p.schedule {
                    Some(s) => Some(s.clone()),
                    None => continue,
                },
                _ if p.ui.is_none() => continue,
                _ => None,
            };
            // Every right, once: a principal and a schedule (a recipient has
            // none, so another `ACT=` value is the same recipient).
            if r.list
                .iter()
                .any(|x| x.uuid == *uuid && x.schedule_uuid == schedule_uuid)
            {
                continue;
            }
            r.list.push(Principal {
                kind,
                uuid: uuid.clone(),
                title: title(uuid, t),
                schedule: schedule_uuid.as_deref().map(schedule_title),
                schedule_uuid,
                members: members.clone(),
            });
        }
    }
    // Each user's groups, nested ones too: once, not once per block.
    let groups_of: Vec<Vec<&loxone_config::access::Group>> = m
        .users
        .iter()
        .map(|u| m.groups_of_transitive(&u.uuid))
        .collect();
    for r in out.values_mut() {
        let holds: BTreeSet<&str> = r.list.iter().map(|p| p.uuid.as_str()).collect();
        let named = |u: &loxone_config::access::User| Named {
            title: title(&u.uuid, &u.title),
            uuid: u.uuid.clone(),
        };
        let direct = |u: &loxone_config::access::User, groups: &[&loxone_config::access::Group]| {
            holds.contains(u.uuid.as_str())
                || groups.iter().any(|g| holds.contains(g.uuid.as_str()))
        };
        let full = |groups: &[&loxone_config::access::Group]| {
            r.kind == RECIPIENTS
                && groups
                    .iter()
                    .any(|g| g.kind.as_deref() == Some(FULL_ACCESS))
        };
        let mut users: Vec<Named> = m
            .users
            .iter()
            .zip(&groups_of)
            .filter(|(u, groups)| direct(u, groups) || full(groups))
            .map(|(u, _)| named(u))
            .collect();
        users.sort();
        let mut inherited: Vec<Named> = m
            .users
            .iter()
            .zip(&groups_of)
            .filter(|(u, groups)| !direct(u, groups) && full(groups))
            .map(|(u, _)| named(u))
            .collect();
        inherited.sort();
        r.inherited = inherited;
        if r.kind == ACCESS {
            for (u, groups) in m.users.iter().zip(&groups_of) {
                let by: BTreeSet<String> = r
                    .list
                    .iter()
                    .filter(|p| p.uuid == u.uuid || groups.iter().any(|g| g.uuid == p.uuid))
                    .filter_map(|p| p.schedule_uuid.clone())
                    .collect();
                if !by.is_empty() {
                    r.schedules.insert(u.uuid.clone(), by);
                }
            }
        }
        r.users = users;
    }
    out
}

/// A user who may still open a door, but under fewer access schedules than
/// before: one was taken away or replaced by another. vislox cannot tell
/// whether the new schedule's times cover the old one's, so this is a loss
/// and warned about like one (#76).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Narrowed {
    pub title: String,
    pub uuid: String,
    /// The schedules (titles) that let the user in before, and now.
    pub from: Vec<String>,
    pub to: Vec<String>,
}

/// A holder whose access schedule changed.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Rescheduled {
    /// As it is now.
    pub principal: Principal,
    /// The schedule's title before.
    pub from: Option<String>,
    /// …and its UUID.
    pub from_uuid: Option<String>,
}

/// How one block's right, or a Daytimer's switching times, differ between
/// two configs: an entry of the review's *Rights and schedules*.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub uuid: String,
    /// [`ACCESS`], [`RECIPIENTS`] or [`SCHEDULE`].
    pub kind: &'static str,
    pub title: String,
    /// The block is new: nothing to compare its rights with.
    pub new_block: bool,
    /// The block is gone: everyone who held the right loses it.
    pub removed_block: bool,
    pub added: Vec<Principal>,
    pub removed: Vec<Principal>,
    pub rescheduled: Vec<Rescheduled>,
    /// The users who hold the right in effect only in the old config, and
    /// only in the new one: what Apply's confirmation warns about.
    pub lose: Vec<Named>,
    pub gain: Vec<Named>,
    /// The users who keep access, but not under every schedule they had:
    /// warned about as a loss too.
    pub narrowed: Vec<Narrowed>,
}

impl Change {
    /// A Daytimer whose switching times changed (the schedule comparison is
    /// `model.rs`'s).
    pub fn schedule(uuid: &str, title: String) -> Change {
        Change {
            uuid: uuid.to_string(),
            kind: SCHEDULE,
            title,
            new_block: false,
            removed_block: false,
            added: Vec::new(),
            removed: Vec::new(),
            rescheduled: Vec::new(),
            lose: Vec::new(),
            gain: Vec::new(),
            narrowed: Vec::new(),
        }
    }

    /// The change in lines, for the change list: `Haustür · access: alice
    /// removed (Always)`.
    #[cfg(test)]
    pub fn lines(&self) -> Vec<String> {
        self.lines_how().into_iter().map(|(_, l)| l).collect()
    }

    /// The lines, each with what it does: `added`, `removed` or `changed`
    /// (the change list colours them so).
    pub fn lines_how(&self) -> Vec<(&'static str, String)> {
        let t = &self.title;
        let k = self.kind;
        let who = |p: &Principal| match p.kind {
            "group" => format!("group {}", p.title),
            _ => p.title.clone(),
        };
        let when = |s: &Option<String>| s.as_ref().map(|s| format!(" ({s})")).unwrap_or_default();
        let mut out = Vec::new();
        if self.removed_block {
            let names: Vec<&str> = self.lose.iter().map(|n| n.title.as_str()).collect();
            let what = match (k, names.len()) {
                (_, 0) => "nobody held a right on it".to_string(),
                (ACCESS, 1) => format!("{} loses access", names[0]),
                (ACCESS, _) => format!("{} lose access", names.join(", ")),
                (_, 1) => format!("{} stops getting it", names[0]),
                _ => format!("{} stop getting it", names.join(", ")),
            };
            out.push(("removed", format!("{t} removed: {what}")));
            return out;
        }
        for p in &self.removed {
            out.push((
                "removed",
                format!("{t} · {k}: {} removed{}", who(p), when(&p.schedule)),
            ));
        }
        for p in &self.added {
            out.push((
                "added",
                format!("{t} · {k}: {} added{}", who(p), when(&p.schedule)),
            ));
        }
        for r in &self.rescheduled {
            out.push((
                "changed",
                format!(
                    "{t} · {k}: {}, {} → {}",
                    who(&r.principal),
                    r.from.as_deref().unwrap_or("—"),
                    r.principal.schedule.as_deref().unwrap_or("—")
                ),
            ));
        }
        out
    }
}

/// How the rights of the blocks differ between `old` and `new` ([`read`]
/// of each), by the block's UUID. A block only `old` has is there too
/// (`removed_block`): whoever held its right loses it. `title` names a block
/// in whichever config has it.
///
/// A right is a principal with a schedule (an access right) or a principal
/// (a recipient). A principal with one right on the block before and one
/// after, under another schedule, is `rescheduled`; any other difference is
/// rights `added` and `removed`.
pub fn changes(
    old: &BTreeMap<String, Rights>,
    new: &BTreeMap<String, Rights>,
    title: impl Fn(&str) -> String,
) -> Vec<Change> {
    let empty = Rights {
        kind: ACCESS,
        list: Vec::new(),
        users: Vec::new(),
        inherited: Vec::new(),
        schedules: BTreeMap::new(),
    };
    let mut uuids: Vec<&String> = new.keys().collect();
    uuids.extend(old.keys().filter(|u| !new.contains_key(*u)));
    let mut out = Vec::new();
    for uuid in uuids {
        let (was, now) = (old.get(uuid), new.get(uuid));
        if was == now {
            continue;
        }
        let kind = now.or(was).map_or(ACCESS, |r| r.kind);
        let (w, n) = (was.unwrap_or(&empty), now.unwrap_or(&empty));
        let key = |p: &Principal| (p.uuid.clone(), p.schedule_uuid.clone());
        let wk: BTreeSet<_> = w.list.iter().map(key).collect();
        let nk: BTreeSet<_> = n.list.iter().map(key).collect();
        let mut added: Vec<Principal> = n
            .list
            .iter()
            .filter(|p| !wk.contains(&key(p)))
            .cloned()
            .collect();
        let mut removed: Vec<Principal> = w
            .list
            .iter()
            .filter(|p| !nk.contains(&key(p)))
            .cloned()
            .collect();
        // One right of a principal for another of the same: a new schedule.
        let mut rescheduled = Vec::new();
        if now.is_some() && was.is_some() {
            let once = |l: &[Principal], u: &str| l.iter().filter(|p| p.uuid == u).count() == 1;
            let pairs: Vec<(usize, usize)> = added
                .iter()
                .enumerate()
                .filter_map(|(i, a)| {
                    let j = removed.iter().position(|r| r.uuid == a.uuid)?;
                    (once(&added, &a.uuid)
                        && once(&removed, &a.uuid)
                        && once(&n.list, &a.uuid)
                        && once(&w.list, &a.uuid))
                    .then_some((i, j))
                })
                .collect();
            for &(i, j) in &pairs {
                rescheduled.push(Rescheduled {
                    principal: added[i].clone(),
                    from: removed[j].schedule.clone(),
                    from_uuid: removed[j].schedule_uuid.clone(),
                });
            }
            let (ai, ri): (BTreeSet<usize>, BTreeSet<usize>) = pairs.into_iter().unzip();
            added = added
                .into_iter()
                .enumerate()
                .filter(|(i, _)| !ai.contains(i))
                .map(|(_, p)| p)
                .collect();
            removed = removed
                .into_iter()
                .enumerate()
                .filter(|(i, _)| !ri.contains(i))
                .map(|(_, p)| p)
                .collect();
        }
        let ids = |l: &[Named]| l.iter().map(|n| n.uuid.clone()).collect::<BTreeSet<_>>();
        let (a, b) = (ids(&w.users), ids(&n.users));
        let lose: Vec<Named> = w
            .users
            .iter()
            .filter(|x| !b.contains(&x.uuid))
            .cloned()
            .collect();
        let gain: Vec<Named> = n
            .users
            .iter()
            .filter(|x| !a.contains(&x.uuid))
            .cloned()
            .collect();
        // Who keeps access under fewer schedules: a schedule's titles from
        // either config.
        let sched_title = |id: &str| {
            w.list
                .iter()
                .chain(&n.list)
                .find(|p| p.schedule_uuid.as_deref() == Some(id))
                .and_then(|p| p.schedule.clone())
                .unwrap_or_else(|| id.to_string())
        };
        let narrowed: Vec<Narrowed> = n
            .users
            .iter()
            .filter_map(|u| {
                let before = w.schedules.get(&u.uuid)?;
                let after = n.schedules.get(&u.uuid).cloned().unwrap_or_default();
                (!before.is_subset(&after)).then(|| Narrowed {
                    title: u.title.clone(),
                    uuid: u.uuid.clone(),
                    from: before.iter().map(|s| sched_title(s)).collect(),
                    to: after.iter().map(|s| sched_title(s)).collect(),
                })
            })
            .collect();
        if now.is_none() && w.list.is_empty() && lose.is_empty() {
            // A block nobody held a right on: its removal is in the change list.
            continue;
        }
        if now.is_some()
            && added.is_empty()
            && removed.is_empty()
            && rescheduled.is_empty()
            && lose.is_empty()
            && gain.is_empty()
            && narrowed.is_empty()
        {
            // Only a member's or a group's title changed.
            continue;
        }
        out.push(Change {
            uuid: uuid.clone(),
            kind,
            title: title(uuid),
            new_block: was.is_none(),
            removed_block: now.is_none(),
            added,
            removed,
            rescheduled,
            lose,
            gain,
            narrowed,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "50000001-0000-0010-ffff504f94000001";
    const BOB: &str = "50000002-0000-0020-ffff504f94000001";
    const CAROL: &str = "50000003-0000-0030-ffff504f94000001";
    const RESIDENTS: &str = "50000004-0000-0040-ffff504f94000001";
    const FULL: &str = "50000005-0000-0050-ffff504f94000001";
    const ALWAYS: &str = "50000006-0000-0060-ffff504f94000001";
    const WORKDAYS: &str = "50000007-0000-0070-ffff504f94000001";
    const DOOR: &str = "50000009-0000-0090-ffff504f94000001";
    const FROST: &str = "5000000a-0000-00a0-ffff504f94000001";

    /// A config with three users, a group of bob and carol, *Full access*
    /// (carol), two access schedules, a Code Touch and a notification. Each
    /// `PH` is given whole.
    fn doc(alice: &str, bob: &str, residents: &str) -> LoxoneDoc {
        let xml = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<ControlList>
	<C Type="LoxCaption" V="178" U="00000000-0000-0001-ffff504f94000001" Title="Users and Permissions">
		<C Type="User" V="178" U="{ALICE}" Title="alice" NFCArr="0A1B2C3D" CodeArr="00112233"><PH>{alice}</PH></C>
		<C Type="User" V="178" U="{BOB}" Title="bob"><PH>{bob}</PH></C>
		<C Type="User" V="178" U="{CAROL}" Title="carol"/>
		<C Type="UserGroup" V="178" U="{RESIDENTS}" Title="Residents" User="{BOB},{CAROL}" UserGroups=""><PH>{residents}</PH></C>
		<C Type="UserGroup" V="178" U="{FULL}" Title="Full access" UgType="4" User="{CAROL}" UserGroups=""/>
		<C Type="SwitchingTimer" V="178" U="{ALWAYS}" Title="Always"/>
		<C Type="SwitchingTimer" V="178" U="{WORKDAYS}" Title="Workdays"/>
		<C Type="Notification" V="178" U="{FROST}" Title="Frost"/>
	</C>
	<C Type="Page" V="178" U="00000000-0000-0002-ffff504f94000001" Title="Eingang">
		<C Type="NfcCodeTouch" V="178" U="{DOOR}" Title="Haustür"/>
	</C>
</ControlList>
"#
        );
        LoxoneDoc::parse(xml.as_bytes()).unwrap()
    }

    fn prm(target: &str, tim: Option<&str>, act: Option<&str>) -> String {
        let tim = tim.map(|t| format!(r#" TIM="{t}""#)).unwrap_or_default();
        let act = act.map(|a| format!(r#" ACT="{a}""#)).unwrap_or_default();
        format!(r#"<PRM REF="{target}"{tim}{act}/>"#)
    }

    fn titles(l: &[Named]) -> Vec<&str> {
        l.iter().map(|n| n.title.as_str()).collect()
    }

    #[test]
    fn rights_are_read_per_block_with_who_holds_them_in_effect() {
        let d = doc(
            &prm(DOOR, Some(ALWAYS), Some("3")),
            &prm(FROST, None, Some("3")),
            &prm(DOOR, Some(WORKDAYS), None),
        );
        let r = read(&d);
        let door = &r[DOOR];
        assert_eq!(door.kind, ACCESS);
        let list: Vec<_> = door
            .list
            .iter()
            .map(|p| (p.kind, p.title.as_str(), p.schedule.as_deref()))
            .collect();
        assert_eq!(
            list,
            [
                ("user", "alice", Some("Always")),
                ("group", "Residents", Some("Workdays"))
            ]
        );
        assert_eq!(door.list[1].members, ["bob", "carol"]);
        // bob and carol through the group.
        assert_eq!(titles(&door.users), ["alice", "bob", "carol"]);
        // alice's ACT= on the Code Touch is a UI right, not a recipient; carol
        // gets the notification as a member of Full access.
        let frost = &r[FROST];
        assert_eq!(frost.kind, RECIPIENTS);
        assert_eq!(frost.list.len(), 1);
        assert_eq!(frost.list[0].title, "bob");
        assert_eq!(titles(&frost.users), ["bob", "carol"]);
        // No credential leaves the module.
        let json = serde_json::to_string(&r).unwrap();
        assert!(
            !json.contains("0A1B2C3D") && !json.contains("00112233"),
            "{json}"
        );
    }

    #[test]
    fn a_removal_names_who_loses_the_right_in_effect() {
        let old = read(&doc(
            &prm(DOOR, Some(ALWAYS), None),
            &prm(DOOR, Some(ALWAYS), None),
            &prm(DOOR, Some(WORKDAYS), None),
        ));
        // alice and bob lose their own rights; bob still opens it through
        // Residents, whose schedule changes.
        let new = read(&doc("", "", &prm(DOOR, Some(ALWAYS), None)));
        let c = changes(&old, &new, |u| {
            if u == DOOR {
                "Haustür".into()
            } else {
                u.into()
            }
        });
        assert_eq!(c.len(), 1, "{c:?}");
        let c = &c[0];
        assert_eq!(
            (c.kind, c.title.as_str(), c.new_block),
            (ACCESS, "Haustür", false)
        );
        let removed: Vec<_> = c.removed.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(removed, ["alice", "bob"]);
        assert!(c.added.is_empty());
        assert_eq!(c.rescheduled.len(), 1);
        assert_eq!(c.rescheduled[0].from.as_deref(), Some("Workdays"));
        assert_eq!(titles(&c.lose), ["alice"]);
        assert!(c.gain.is_empty());
        assert_eq!(
            c.lines(),
            [
                "Haustür · access: alice removed (Always)",
                "Haustür · access: bob removed (Always)",
                "Haustür · access: group Residents, Workdays → Always",
            ]
        );
    }

    #[test]
    fn recipients_added_and_removed_and_no_change_is_none() {
        let old = read(&doc("", &prm(FROST, None, Some("1")), ""));
        let new = read(&doc(&prm(FROST, None, Some("3")), "", ""));
        let c = changes(&old, &new, |_| "Frost".into());
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].kind, RECIPIENTS);
        assert_eq!(
            c[0].lines(),
            [
                "Frost · recipients: bob removed",
                "Frost · recipients: alice added"
            ]
        );
        assert_eq!(titles(&c[0].lose), ["bob"]);
        assert_eq!(titles(&c[0].gain), ["alice"]);
        // Another ACT= value is the same recipient.
        let again = read(&doc("", &prm(FROST, None, Some("3")), ""));
        assert!(changes(&old, &again, |_| String::new()).is_empty());
    }

    #[test]
    fn a_second_right_of_one_principal_is_kept_and_compared() {
        // alice may open the door always and on workdays: two `<PRM>`s.
        let two = prm(DOOR, Some(ALWAYS), None) + &prm(DOOR, Some(WORKDAYS), None);
        let old = read(&doc(&two, "", ""));
        let door = &old[DOOR];
        let got: Vec<_> = door
            .list
            .iter()
            .map(|p| (p.title.as_str(), p.schedule_uuid.as_deref()))
            .collect();
        assert_eq!(got, [("alice", Some(ALWAYS)), ("alice", Some(WORKDAYS))]);
        // The workdays right goes: a removal, not a reschedule, and she
        // still opens the door.
        let new = read(&doc(&prm(DOOR, Some(ALWAYS), None), "", ""));
        let c = changes(&old, &new, |_| "Haustür".into());
        assert_eq!(c.len(), 1, "{c:?}");
        assert!(c[0].rescheduled.is_empty() && c[0].added.is_empty());
        assert_eq!(c[0].removed.len(), 1);
        assert_eq!(c[0].removed[0].schedule_uuid.as_deref(), Some(WORKDAYS));
        assert!(c[0].lose.is_empty());
        assert_eq!(c[0].lines(), ["Haustür · access: alice removed (Workdays)"]);
        // One right for another of the same principal is a reschedule, by
        // the schedule's UUID.
        let moved = read(&doc(&prm(DOOR, Some(WORKDAYS), None), "", ""));
        let c = changes(&new, &moved, |_| "Haustür".into());
        assert_eq!(c[0].rescheduled.len(), 1, "{c:?}");
        assert_eq!(c[0].rescheduled[0].from_uuid.as_deref(), Some(ALWAYS));
        assert_eq!(
            c[0].rescheduled[0].principal.schedule_uuid.as_deref(),
            Some(WORKDAYS)
        );
    }

    #[test]
    fn a_removed_block_takes_its_rights_with_it() {
        let old = read(&doc(
            &prm(DOOR, Some(ALWAYS), None),
            &prm(FROST, None, Some("3")),
            &prm(DOOR, Some(WORKDAYS), None),
        ));
        let mut new = old.clone();
        new.remove(DOOR);
        new.remove(FROST);
        let c = changes(&old, &new, |u| {
            if u == DOOR {
                "Haustür".into()
            } else {
                "Frost".into()
            }
        });
        let got: Vec<_> = c
            .iter()
            .map(|c| (c.kind, c.removed_block, c.lines()))
            .collect();
        assert_eq!(
            got,
            [
                (
                    ACCESS,
                    true,
                    vec!["Haustür removed: alice, bob, carol lose access".to_string()]
                ),
                (
                    RECIPIENTS,
                    true,
                    vec!["Frost removed: bob, carol stop getting it".to_string()]
                ),
            ]
        );
        assert_eq!(titles(&c[0].lose), ["alice", "bob", "carol"]);
        assert_eq!(c[0].removed.len(), 2);
        // A block nobody held a right on goes unnoticed here.
        let none = read(&doc("", "", ""));
        let mut gone = none.clone();
        gone.remove(DOOR);
        assert!(changes(&none, &gone, |_| String::new()).is_empty());
    }

    /// A schedule replaced is a loss of opening times, even though the user
    /// still may open the door; another schedule added is not (#76).
    #[test]
    fn a_narrower_schedule_is_a_loss() {
        let at = |who: &str, d: &LoxoneDoc| {
            changes(
                &read(&doc("", "", &prm(DOOR, Some(ALWAYS), None))),
                &read(d),
                |u| u.to_string(),
            )
            .into_iter()
            .find(|c| c.uuid == DOOR)
            .map(|c| {
                c.narrowed
                    .into_iter()
                    .filter(|n| n.title == who)
                    .map(|n| (n.from, n.to))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        };
        // Residents (bob, carol) on workdays instead of always.
        let narrower = doc("", "", &prm(DOOR, Some(WORKDAYS), None));
        assert_eq!(
            at("bob", &narrower),
            [(vec!["Always".to_string()], vec!["Workdays".to_string()])]
        );
        let c = changes(
            &read(&doc("", "", &prm(DOOR, Some(ALWAYS), None))),
            &read(&narrower),
            |u| u.to_string(),
        );
        assert!(c[0].lose.is_empty(), "nobody loses the door, only times");
        // bob on workdays too, the group still always: nothing taken.
        let wider = doc(
            "",
            &prm(DOOR, Some(WORKDAYS), None),
            &prm(DOOR, Some(ALWAYS), None),
        );
        assert!(at("bob", &wider).is_empty());
    }
}
