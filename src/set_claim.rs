//! Claims on a block's whole set, shared by several blocks: a DayTimer's
//! switching times (`schedule`, #73), a lighting controller's moods (`moods`,
//! #74), who may open an NFC Code Touch (`access`, #76) and who gets a push
//! notification (`recipients`, #77), and the next one (#78 tiles).
//!
//! Lox Logic writes each as a named set and a claim per block
//! (`<slug>.<keyword> = <set>`). Several blocks may claim one set; a block
//! claims one set; dropping a claim leaves the block as the base has it.
//! vislox keeps its sets and claims in its file of claims (`edits.loxl`). One
//! op per kind gives a block a set's content, a shared set, or no claim, and
//! this module does what is the same for every kind:
//!
//! - resolve the block (an extern of the file, or a block of vislox's files),
//!   refusing one the rest of the module manages or claims;
//! - choose the set: the one named, the block's own when no other block
//!   claims it, else a new one (`vx_<name>_<stem>`);
//! - write it, checked by the kind for every block that claims it, and refuse
//!   a claimant the pending config no longer has (its stale claim is dropped
//!   with the op and no value);
//! - make `previous` a whole inverse: the block's claim as it was, and the
//!   content of every other set the op rewrote that other blocks still claim
//!   (`restore`), so an undo gives them back too;
//! - and the section of `GET /api/edits` that lists them.

use crate::edit::{self, Applied, Claims, Ctx, Files, Lock, Op};
use loxone_config::LoxoneDoc;
use loxone_config::logic::Item;
use loxone_config::xml::Element;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Debug, Display};
use std::marker::PhantomData;

/// The blocks the rest of the module claims, per kind (its keyword:
/// `schedule`, `moods`, `members`, `access`, `recipients`): UUID (lowercase) → the claim's
/// `file:line`.
pub type Claimed = BTreeMap<&'static str, BTreeMap<String, String>>;

/// What a statement of vislox's file of claims is to a [`SetKind`].
pub enum SetItem<L> {
    /// `<keyword> <name> … end`.
    Set(String, Vec<L>),
    /// `<slug>.<keyword> = <set>`.
    Claim(String, String),
}

/// Every kind of claim vislox writes on a block's whole content, in the
/// order its file lists them. A new kind is one entry here (and its slot in
/// [`Claims`]): reading and writing the file, the names a new set must not
/// take, the edit's checks before and after the compile, the refusals of
/// Remove and Hand back, and `GET /api/edits` go through this list.
pub static KINDS: &[&dyn ClaimKind] = &[
    &Reg::<crate::sched_edit::SchedKind>(PhantomData),
    &Reg::<crate::mood_edit::MoodKind>(PhantomData),
    &Reg::<crate::access_edit::AccessKind>(PhantomData),
    &Reg::<crate::recipients_edit::RecipientsKind>(PhantomData),
    &crate::member_edit::MemberKind,
];

/// One kind of claim, as the code that is the same for every kind sees it.
pub trait ClaimKind: Sync {
    /// `schedule`, `moods`, `access`, `members`: the claim's port.
    fn keyword(&self) -> &'static str;
    /// What it claims, in messages: `switching times`, `members`.
    fn what(&self) -> &'static str;
    /// Whether it writes named sets, whose new names must not be the
    /// module's.
    fn makes_sets(&self) -> bool;
    /// The block an op of this kind is on, and whether it only drops
    /// vislox's claim.
    fn op<'a>(&self, op: &'a Op) -> Option<(&'a str, bool)>;
    /// The sets an op of this kind writes back (`restore`).
    fn restored(&self, op: &Op) -> Vec<String>;
    fn is_empty(&self, c: &Claims) -> bool;
    /// Its part of the file: the sets, then the claims.
    fn render(&self, c: &Claims) -> String;
    /// What the file claims on the block `slug`, said after "vislox's file
    /// claims ", and the pronoun for it (`them`, `it`).
    fn on(&self, c: &Claims, slug: &str) -> Option<(String, &'static str)>;
    /// The names of its sets (one namespace for every kind).
    fn set_names<'a>(&self, c: &'a Claims) -> Vec<&'a str>;
    /// Read a statement of the file into `c`; `None`: not this kind's.
    fn read(&self, c: &mut Claims, item: &Item) -> Option<Result<(), String>>;
    /// What a read file must hold together (a claim names one of its sets).
    fn check_read(&self, c: &Claims) -> Result<(), String>;
    /// Each claim's statement and a slug it names, for the check that every
    /// slug is an extern of the file or a block of vislox's files.
    fn names(&self, c: &Claims) -> Vec<(String, String)>;
    /// After the compile: the blocks among `uuids` (and those sharing sets
    /// with them, or on the sets `restored`) the config does not have as
    /// their claim says.
    fn missed(
        &self,
        files: &Files,
        out: &LoxoneDoc,
        lock: &Lock,
        uuids: &[String],
        restored: &[String],
    ) -> Vec<String>;
    /// Its keys of `GET /api/edits`, and their values.
    fn edits_section(
        &self,
        now: &Files,
        was: &Files,
        lock: &Lock,
        pending: Option<&LoxoneDoc>,
        deployed: Option<&LoxoneDoc>,
    ) -> Vec<(&'static str, Value)>;
    /// For the browser: the keys a set kind's section uses (null for one
    /// that has no sets).
    fn keys(&self) -> Value;
}

/// The kind of claim `op` is, if it is one.
pub fn kind_of(op: &Op) -> Option<&'static dyn ClaimKind> {
    KINDS.iter().copied().find(|k| k.op(op).is_some())
}

/// A [`SetKind`] as a [`ClaimKind`].
pub struct Reg<K>(pub PhantomData<fn() -> K>);

impl<K: SetKind> ClaimKind for Reg<K> {
    fn keyword(&self) -> &'static str {
        K::KEYWORD
    }
    fn what(&self) -> &'static str {
        K::WHAT
    }
    fn makes_sets(&self) -> bool {
        true
    }
    fn op<'a>(&self, op: &'a Op) -> Option<(&'a str, bool)> {
        K::op(op).map(|(u, v)| {
            let claiming = v.is_some_and(|v| {
                let (set, content, _) = K::parts(v);
                set.is_some() || content.is_some()
            });
            (u, !claiming)
        })
    }
    fn restored(&self, op: &Op) -> Vec<String> {
        K::op(op)
            .and_then(|(_, v)| v)
            .map(|v| K::parts(v).2.keys().cloned().collect())
            .unwrap_or_default()
    }
    fn is_empty(&self, c: &Claims) -> bool {
        K::store(c).is_empty()
    }
    fn render(&self, c: &Claims) -> String {
        K::store(c).render()
    }
    fn on(&self, c: &Claims, slug: &str) -> Option<(String, &'static str)> {
        K::store(c)
            .claims
            .contains_key(slug)
            .then(|| (format!("its {}", K::WHAT), "them"))
    }
    fn set_names<'a>(&self, c: &'a Claims) -> Vec<&'a str> {
        K::store(c).sets.keys().map(String::as_str).collect()
    }
    fn read(&self, c: &mut Claims, item: &Item) -> Option<Result<(), String>> {
        let s = K::store_mut(c);
        Some(match K::read_item(item)? {
            SetItem::Set(name, lines) => {
                if s.sets.insert(name.clone(), lines).is_some() {
                    Err(format!("a second `{} {name}`", K::KEYWORD))
                } else {
                    Ok(())
                }
            }
            SetItem::Claim(slug, set) => {
                if s.claims.insert(slug.clone(), set).is_some() {
                    Err(format!("a second `{slug}.{}`", K::KEYWORD))
                } else {
                    Ok(())
                }
            }
        })
    }
    fn check_read(&self, c: &Claims) -> Result<(), String> {
        let s = K::store(c);
        for (t, set) in &s.claims {
            if !s.sets.contains_key(set) {
                return Err(format!("`{t}.{} = {set}` without its set", K::KEYWORD));
            }
        }
        Ok(())
    }
    fn names(&self, c: &Claims) -> Vec<(String, String)> {
        K::store(c)
            .claims
            .keys()
            .map(|t| (format!("{t}.{}", K::KEYWORD), t.clone()))
            .collect()
    }
    fn missed(
        &self,
        files: &Files,
        out: &LoxoneDoc,
        lock: &Lock,
        uuids: &[String],
        restored: &[String],
    ) -> Vec<String> {
        K::missed(files, out, lock, uuids, restored)
    }
    fn edits_section(
        &self,
        now: &Files,
        was: &Files,
        lock: &Lock,
        pending: Option<&LoxoneDoc>,
        deployed: Option<&LoxoneDoc>,
    ) -> Vec<(&'static str, Value)> {
        let (entries, sets) = edits_section::<K>(now, was, lock, pending, deployed);
        vec![(K::ENTRIES_KEY, entries), (K::SETS_KEY, sets)]
    }
    fn keys(&self) -> Value {
        json!({
            "op": K::KEYWORD,
            "entries": K::ENTRIES_KEY,
            "sets": K::SETS_KEY,
            "content": K::CONTENT_KEY,
            "members": K::MEMBERS_KEY,
            "what": K::WHAT,
        })
    }
}

/// One kind of set claim.
pub trait SetKind: Clone + Debug + Default + PartialEq + 'static {
    /// A line of a set, as lox's parser reads it.
    type Line: Clone + Debug + Display + PartialEq;
    /// A set's content as an op carries it.
    type Content: Clone + Debug + PartialEq + Serialize + DeserializeOwned;
    /// The op's value.
    type Val: Clone + Debug + Default + PartialEq + Serialize;
    /// `schedule`, `moods`: the set's keyword and the claim's port.
    const KEYWORD: &'static str;
    /// The block type that takes the claim.
    const BLOCK: &'static str;
    /// What the set holds, in messages: `switching times`, `moods`.
    const WHAT: &'static str;
    /// The last word of a new set's name: `vx_uhr_schedule`.
    const STEM: &'static str;
    /// The comment above the sets in the file.
    const HEADER: &'static str;
    /// The keys of `GET /api/edits`: an entry's content, a set's blocks,
    /// the list of entries and the list of sets.
    const CONTENT_KEY: &'static str;
    const MEMBERS_KEY: &'static str;
    const ENTRIES_KEY: &'static str;
    const SETS_KEY: &'static str;
    /// What reading a set's content against one config needs, worked out
    /// once per config rather than once per set.
    type Index<'a>;

    fn store(c: &Claims) -> &SetStore<Self>;
    fn store_mut(c: &mut Claims) -> &mut SetStore<Self>;
    /// Why `name` takes no such claim.
    fn wrong_type(name: &str) -> String;

    /// The parts of an op's value: set, content, sets to write back first.
    fn parts(v: &Self::Val) -> Parts<'_, Self>;
    fn make(
        set: Option<String>,
        content: Option<Self::Content>,
        restore: BTreeMap<String, Self::Content>,
    ) -> Self::Val;
    fn index<'a>(doc: &'a LoxoneDoc, lock: &'a Lock) -> Self::Index<'a>;
    /// A set's lines as an op carries them; `None` when they cannot be read
    /// against the config `idx` was made of.
    fn content_of(lines: &[Self::Line], idx: &Self::Index<'_>) -> Option<Self::Content>;
    /// A statement of the file, if it is this kind's and vislox's (`vx_…`).
    fn read_item(item: &Item) -> Option<SetItem<Self::Line>>;
    /// The block an op of this kind is on, and its value.
    fn op(op: &Op) -> Option<(&str, Option<&Self::Val>)>;
    /// After the compile: see [`ClaimKind::missed`].
    fn missed(
        files: &Files,
        out: &LoxoneDoc,
        lock: &Lock,
        uuids: &[String],
        restored: &[String],
    ) -> Vec<String>;
    /// Whether a block has other content in `a` than in `b`: the test,
    /// worked out once per pair of configs.
    fn differ<'a>(a: &'a LoxoneDoc, b: &'a LoxoneDoc) -> Box<dyn Fn(&str) -> bool + 'a>;
    /// An op's content as lines, checked for its blocks (`members`: slug and
    /// element; the block about to claim it first). `idx` is the pending
    /// config's [`SetKind::index`], read once per op.
    fn lines_of(
        content: &Self::Content,
        ctx: &Ctx,
        idx: &Self::Index<'_>,
        members: &[(String, &Element)],
    ) -> Result<Vec<Self::Line>, String>;
    /// Check a set's lines for every block that claims it.
    fn check(
        lines: &[Self::Line],
        ctx: &Ctx,
        idx: &Self::Index<'_>,
        set: &str,
        members: &[(String, &Element)],
    ) -> Result<(), String>;
}

/// What [`SetKind::parts`] gives.
pub type Parts<'a, K> = (
    Option<&'a String>,
    Option<&'a <K as SetKind>::Content>,
    &'a BTreeMap<String, <K as SetKind>::Content>,
);

/// The sets and claims of one kind in vislox's file of claims.
#[derive(Clone, PartialEq, Debug)]
pub struct SetStore<K: SetKind> {
    /// Set name → its lines, as lox reads them.
    pub sets: BTreeMap<String, Vec<K::Line>>,
    /// The slug of a block (an extern of the file, or a block of vislox's
    /// files) → the set it claims.
    pub claims: BTreeMap<String, String>,
}

impl<K: SetKind> Default for SetStore<K> {
    fn default() -> Self {
        SetStore {
            sets: BTreeMap::new(),
            claims: BTreeMap::new(),
        }
    }
}

impl<K: SetKind> SetStore<K> {
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty() && self.claims.is_empty()
    }

    /// The blocks that claim `set`.
    pub fn claimants(&self, set: &str) -> Vec<&str> {
        self.claims
            .iter()
            .filter(|(_, s)| s.as_str() == set)
            .map(|(t, _)| t.as_str())
            .collect()
    }

    /// Drop the sets no block claims.
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
        out.push('\n');
        out.push_str(K::HEADER);
        for (name, lines) in &self.sets {
            out.push_str(&format!("{} {name}\n", K::KEYWORD));
            for l in lines {
                out.push_str(&format!("\t{l}\n"));
            }
            out.push_str("end\n");
        }
        out.push('\n');
        for (t, s) in &self.claims {
            out.push_str(&format!("{t}.{} = {s}\n", K::KEYWORD));
        }
        out
    }
}

/// The block a slug of vislox's files names: an extern of the file of claims
/// (`vx_<uuid>`), or a block vislox added or adopted.
pub fn member_of(files: &Files, lock: &Lock, slug: &str) -> Option<String> {
    files
        .claims
        .0
        .keys()
        .find(|u| edit::slug(u) == slug)
        .cloned()
        .or_else(|| lock.uuid_of(slug).map(String::from))
}

/// The slug vislox's files name the block `uuid` by.
pub fn target_of(files: &Files, lock: &Lock, uuid: &str) -> String {
    lock.slug_of(uuid)
        .filter(|t| files.block(t).is_some())
        .map(String::from)
        .unwrap_or_else(|| edit::slug(uuid))
}

/// Whether a new set may not be named `s`: a name of the module, of the
/// lockfile (its tombstones too) or of vislox's files, its sets of every kind
/// among them (they share one namespace).
pub fn taken(files: &Files, ctx: &Ctx, s: &str) -> bool {
    ctx.names.contains(s)
        || ctx.lock.slugs.contains_key(s)
        || ctx.lock.tombstones.contains(s)
        || ctx.lock.externals.contains_key(s)
        || files.names(s)
        || KINDS
            .iter()
            .any(|k| k.set_names(&files.claims).contains(&s))
        || files.claims.0.keys().any(|u| edit::slug(u) == s)
}

/// What `files` give the block `target`: its set and the set's content
/// (none when it cannot be read against `doc`).
pub fn current<K: SetKind>(files: &Files, target: &str, idx: &K::Index<'_>) -> Option<K::Val> {
    let s = K::store(&files.claims);
    let set = s.claims.get(target)?;
    Some(K::make(
        Some(set.clone()),
        s.sets.get(set).and_then(|l| K::content_of(l, idx)),
        BTreeMap::new(),
    ))
}

/// Write `content` into vislox's set `set` (or, with none, check the set as
/// it is), checked for every block that claims it and `also`, about to claim
/// it. A claimant the pending config does not have is refused, named: its
/// claim is stale (the block was deleted in Loxone Config), and the kind's op
/// with its UUID and no value drops it.
fn write_set<K: SetKind>(
    files: &mut Files,
    ctx: &Ctx,
    idx: &K::Index<'_>,
    set: &str,
    content: Option<&K::Content>,
    also: Option<(&str, &Element)>,
) -> Result<(), String> {
    let mut members: Vec<(String, &Element)> =
        also.map(|(t, e)| (t.to_string(), e)).into_iter().collect();
    for t in K::store(&files.claims).claimants(set) {
        if also.is_some_and(|(a, _)| a == t) {
            continue;
        }
        let u = member_of(files, ctx.lock, t);
        match u.as_deref().and_then(|u| edit::find(&ctx.doc.xml.root, u)) {
            Some(e) => members.push((t.to_string(), e)),
            None => {
                return Err(format!(
                    "`{set}` is also claimed for {}, which the pending config does not have (deleted \
                     in Loxone Config?); drop that stale claim first (Drop its claim, or the `{}` op \
                     with its UUID and no value)",
                    u.as_deref().unwrap_or(t),
                    K::KEYWORD
                ));
            }
        }
    }
    if let Some(c) = content {
        let lines = K::lines_of(c, ctx, idx, &members)?;
        K::store_mut(&mut files.claims)
            .sets
            .insert(set.to_string(), lines);
    }
    let lines = K::store(&files.claims).sets.get(set).ok_or_else(|| {
        format!(
            "vislox has no {} set `{set}`; give the {}",
            K::KEYWORD,
            K::WHAT
        )
    })?;
    K::check(lines, ctx, idx, set, &members)
}

/// Apply an op of kind `K`: give the block `uuid` the set `value` says, or
/// drop vislox's claim (`None`, or neither set nor content). Its `previous`
/// undoes it whole: the block's claim as it was, and the content of every
/// other set the op rewrote that other blocks still claim (`restore`).
pub fn apply<K: SetKind>(
    files: &mut Files,
    ctx: &Ctx,
    uuid: &str,
    value: Option<&K::Val>,
) -> Result<Applied, String> {
    let claiming = value.is_some_and(|v| {
        let (set, content, _) = K::parts(v);
        set.is_some() || content.is_some()
    });
    let el = edit::find(&ctx.doc.xml.root, uuid);
    let target = target_of(files, ctx.lock, uuid);
    // A block the pending config lost: only its stale claim can go.
    let el = match el {
        Some(e) => Some(e),
        None if !claiming && K::store(&files.claims).claims.contains_key(&target) => None,
        None => return Err(format!("no object {uuid} in the pending config")),
    };
    let name = el.map_or_else(
        || uuid.to_string(),
        |e| e.attr_decoded("Title").unwrap_or_default().into_owned(),
    );
    // Dropping vislox's claim is always let through, whatever the block has
    // become (another type, or managed by the module): a claim left behind
    // would break every compile.
    if let Some(e) = el.filter(|_| claiming) {
        if e.attr("Type") != Some(K::BLOCK) {
            return Err(K::wrong_type(&name));
        }
        if let Some(s) = ctx.lock.slug_of(uuid)
            && files.block(s).is_none()
        {
            return Err(format!(
                "{name}: the module manages this block; change its {} in its .loxl file",
                K::WHAT
            ));
        }
    }
    let before = K::store(&files.claims).clone();
    let idx = K::index(ctx.doc, ctx.lock);
    let mut previous = current::<K>(files, &target, &idx);
    // Sets of vislox's written back first (an undo's).
    if let Some(v) = value {
        for (set, content) in K::parts(v).2 {
            if !K::store(&files.claims).sets.contains_key(set) {
                return Err(format!(
                    "{name}: vislox's set `{set}` is gone; its {} cannot be put back",
                    K::WHAT
                ));
            }
            write_set::<K>(files, ctx, &idx, set, Some(content), None)?;
        }
    }
    match (value, el) {
        (Some(v), Some(el)) if claiming => {
            let (set, content, _) = K::parts(v);
            // A block takes one set. Dropping vislox's claim is always let
            // through: it is how a block the module claims too is mended.
            if let Some(at) = ctx.claimed(K::KEYWORD).get(&uuid.to_ascii_lowercase()) {
                return Err(format!(
                    "{name}: {at} claims its {}; change them there (a {} takes one {} set)",
                    K::WHAT,
                    K::BLOCK,
                    K::KEYWORD
                ));
            }
            let s = K::store(&files.claims);
            let set = match set {
                Some(n) => n.trim().to_string(),
                None => match s.claims.get(&target) {
                    Some(own) if s.claimants(own) == [target.as_str()] => own.clone(),
                    _ => {
                        crate::adopt::default_slug(&format!("{name} {}", K::STEM), K::BLOCK, &|x| {
                            taken(files, ctx, x)
                        })
                    }
                },
            };
            if !s.sets.contains_key(&set) {
                crate::adopt::check_slug(&set).map_err(|e| format!("The set's name: {e}"))?;
                if taken(files, ctx, &set) {
                    return Err(format!(
                        "The set's name: `{set}` is taken in the module; pick another"
                    ));
                }
                if content.is_none() {
                    return Err(format!(
                        "{name}: vislox has no {} set `{set}`; give the {}",
                        K::KEYWORD,
                        K::WHAT
                    ));
                }
            }
            write_set::<K>(files, ctx, &idx, &set, content, Some((&target, el))).map_err(|e| {
                if e.starts_with(&name) {
                    e
                } else {
                    format!("{name}: {e}")
                }
            })?;
            K::store_mut(&mut files.claims)
                .claims
                .insert(target.clone(), set);
            // An extern of the file names a block Loxone Config drew.
            if target == edit::slug(uuid) {
                let c = files.claims.0.entry(uuid.to_string()).or_default();
                c.ty = K::BLOCK.into();
            }
        }
        _ => {
            K::store_mut(&mut files.claims).claims.remove(&target);
        }
    }
    K::store_mut(&mut files.claims).prune();
    files.claims.prune();
    // The sets the op rewrote that other blocks still claim: their old
    // content goes into the undo, unless the undo's own set is that set.
    let own = previous.as_ref().and_then(|p| K::parts(p).0.cloned());
    let mut restore = BTreeMap::new();
    let now = K::store(&files.claims);
    for (set, lines) in &before.sets {
        let others = now.claimants(set).iter().any(|t| *t != target);
        if now.sets.get(set).is_some_and(|n| n != lines) && others && own.as_ref() != Some(set) {
            let c = K::content_of(lines, &idx)
                .ok_or_else(|| format!("`{set}`: its {} cannot be read", K::WHAT))?;
            restore.insert(set.clone(), c);
        }
    }
    if !restore.is_empty() {
        let (set, content) = previous
            .as_ref()
            .map(|p| {
                let (s, c, _) = K::parts(p);
                (s.cloned(), c.cloned())
            })
            .unwrap_or_default();
        previous = Some(K::make(set, content, restore));
    }
    Ok(Applied {
        previous: json!(previous),
        added: None,
    })
}

/// The sets an op request makes lox write: those its blocks claim after it,
/// and those it writes back (`restored`).
pub fn sets_written<'a, K: SetKind>(
    files: &'a Files,
    lock: &Lock,
    uuids: &[String],
    restored: &'a [String],
) -> BTreeSet<&'a str> {
    let s = K::store(&files.claims);
    let mut sets: BTreeSet<&str> = restored.iter().map(String::as_str).collect();
    for u in uuids {
        if let Some(set) = s.claims.get(&target_of(files, lock, u)) {
            sets.insert(set);
        }
    }
    sets
}

/// `GET /api/edits`' section for kind `K`. Per block that vislox's file
/// claims now or claimed in the deployed revision: its set and the set's
/// content now (`set`, [`SetKind::CONTENT_KEY`]) and then (`deployed`, the
/// value a Revert sends), the other blocks sharing its set (`shared`), and
/// whether the pending config has it otherwise than the deployed one
/// (`pending`, by `differs`). And vislox's sets, each with its blocks
/// ([`SetKind::MEMBERS_KEY`]), its content, and the claimants the config shown
/// does not have (`stale`).
pub fn edits_section<K: SetKind>(
    now: &Files,
    was: &Files,
    lock: &Lock,
    pending: Option<&LoxoneDoc>,
    deployed: Option<&LoxoneDoc>,
) -> (Value, Value) {
    let Some(doc) = pending.or(deployed) else {
        return (json!([]), json!([]));
    };
    // Each config read once, not once per block.
    let idx = K::index(doc, lock);
    let differs = match (pending, deployed) {
        (Some(p), Some(d)) => Some(K::differ(p, d)),
        _ => None,
    };
    // Block UUID → its slug in vislox's files now, and in the deployed ones.
    let mut blocks: BTreeMap<String, (Option<&str>, Option<&str>)> = BTreeMap::new();
    for (files, now_side) in [(now, true), (was, false)] {
        for t in K::store(&files.claims).claims.keys() {
            if let Some(u) = member_of(files, lock, t) {
                let slot = blocks.entry(u).or_default();
                if now_side {
                    slot.0 = Some(t)
                } else {
                    slot.1 = Some(t)
                }
            }
        }
    }
    let store = K::store(&now.claims);
    let uuids_of = |set: &str, but: Option<&str>| -> Vec<String> {
        store
            .claimants(set)
            .into_iter()
            .filter(|t| Some(*t) != but)
            .filter_map(|t| member_of(now, lock, t))
            .collect()
    };
    let list: Vec<Value> = blocks
        .into_iter()
        .map(|(u, (tn, tw))| {
            let cur = tn.and_then(|t| current::<K>(now, t, &idx));
            let dep = tw.and_then(|t| current::<K>(was, t, &idx));
            let (set, content) = cur.as_ref().map_or((None, None), |c| {
                let (s, c, _) = K::parts(c);
                (s.cloned(), c.cloned())
            });
            let mut v = json!({
                "uuid": u,
                "set": set,
                "deployed": dep,
                "shared": set.as_deref().map(|s| uuids_of(s, tn)).unwrap_or_default(),
                // As for a value: with something pending, a revision that
                // was never deployed differs from it.
                "pending": match (&differs, pending) {
                    (Some(f), _) => f(&u),
                    (None, Some(_)) => true,
                    (None, None) => false,
                },
            });
            v[K::CONTENT_KEY] = json!(content);
            v
        })
        .collect();
    let sets: Vec<Value> = store
        .sets
        .iter()
        .map(|(name, lines)| {
            let members = uuids_of(name, None);
            let stale: Vec<&String> = members
                .iter()
                .filter(|u| edit::find(&doc.xml.root, u).is_none())
                .collect();
            let mut v = json!({ "name": name, "stale": stale });
            v[K::MEMBERS_KEY] = json!(members);
            v[K::CONTENT_KEY] = json!(K::content_of(lines, &idx));
            v
        })
        .collect();
    (json!(list), json!(sets))
}
