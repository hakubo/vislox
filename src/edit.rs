//! Editing from Properties: the edit model, what can be edited, and the
//! file the edits are written to. The design is in `docs/editing.md`.
//!
//! An edit is a list of [`Op`]s against a base revision (the blob id of the
//! pending config the browser showed). vislox writes them as Lox Logic claims
//! into a file of its own in the project's module ([`FILE`]), and
//! `lox logic compile` turns the module into the pending `out.Loxone`. So an
//! edit shows as the pending revision, and Apply, Discard and Sync work on it
//! as on any other compile. Nothing here talks to the Miniserver.
//!
//! The claims are `extern` statements pinned by UUID, plus one assignment per
//! value: `x.In2 = 100` (an input's constant), `x.title = "…"`, `x.MinTime = 500`
//! (a setting of a physical input or output). lox records each original value in
//! the lockfile and restores it when the statement leaves the module, so
//! dropping a claim hands the value back to Loxone Config.

use crate::blocks::{self, NewBlock, PageFile};
use loxone_config::connectors::{self, PortDir, PortUnit};
use loxone_config::layout::GRID;
use loxone_config::lock::Layout;
use loxone_config::logic::{ArgItem, Attr, BindingKind, Item, MatchSpec, Module, Value};
use loxone_config::port_names;
use loxone_config::xml::Element;
use loxone_config::{LoxoneDoc, logic};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

/// vislox's own file in the module directory, for the values it claims.
/// The rest of the module is never touched. The blocks vislox adds (#59)
/// are in files of their own next to it, one per page ([`crate::blocks`]).
pub const FILE: &str = "vislox/edits.loxl";

/// vislox's directory in the module: [`FILE`] and the files of new blocks.
pub const DIR: &str = "vislox";

/// The slugs of vislox's files start with this; nothing else in a module should.
const SLUG_PREFIX: &str = crate::blocks::SLUG_PREFIX;

/// One edit. `value: None` drops vislox's claim on the value: lox puts
/// back what Loxone Config had before the claim (see the module comment).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// The object's name (`Title=`).
    Title { uuid: String, value: Option<String> },
    /// The room it is in (`<IoData Pr=…>`), by the room's title.
    Room { uuid: String, value: Option<String> },
    /// Its category (`<IoData Cr=…>`), by the category's title.
    Category { uuid: String, value: Option<String> },
    /// The constant of an unwired input (`Def=` of connector `port`, its `K=`).
    Param {
        uuid: String,
        port: String,
        value: Option<String>,
    },
    /// A setting of a physical input or output, or of a device (`MinTime=`,
    /// `Desc=`, `Installation=`, …).
    Setting {
        uuid: String,
        name: String,
        value: Option<String>,
    },
    /// A new block of `type` on the page with UUID `page` (#59). lox mints
    /// its UUIDs and places it; the reply names it (`added`).
    AddBlock {
        page: String,
        #[serde(rename = "type")]
        ty: String,
        /// Its name; none shows the slug vislox picks.
        #[serde(default)]
        title: Option<String>,
        /// Port key → the constant of an input or parameter. A port left
        /// out gets the value Loxone Config places the block with.
        #[serde(default)]
        params: BTreeMap<String, String>,
        /// Setting (`Formula`, `Type`, `At`, …) → value.
        #[serde(default)]
        settings: BTreeMap<String, String>,
        /// Wires (#61) to draw with it; an empty UUID is this block. The
        /// `previous` of a `remove_block` carries the wires it took out.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        wires: Vec<Ends>,
        /// The UUID of the block this adds again: set in the `previous` of a
        /// `remove_block`. lox mints new UUIDs, so the reply's `added` names
        /// it next to the new one, and undo and redo (#62) can follow the
        /// block. It changes nothing in what is written.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        restores: Option<String>,
    },
    /// Delete a block of vislox's files: its declaration goes, and
    /// `removed <slug>` tells lox. One added here and not deployed yet is
    /// simply taken out: its `previous` is the `add_block` that adds it again
    /// (with new UUIDs). A deployed or adopted one (#80) is deleted from the
    /// config, and the Miniserver on Apply: its `previous` is the
    /// `restore_block` that brings it back with its UUIDs.
    RemoveBlock { uuid: String },
    /// Bring back a block `remove_block` deleted while lox still keeps its
    /// tombstone (#80): its declaration as it was, its lockfile entry (so it
    /// keeps its UUIDs, layout and wires) and the wires drawn here at it.
    /// Only the `previous` of a `remove_block` makes one. Its `previous` is
    /// the `remove_block`.
    RestoreBlock {
        uuid: String,
        /// The page it is on (its file's).
        page: String,
        /// Adopted from Loxone Config (#72): its file's *adopted* section.
        #[serde(default)]
        adopted: bool,
        /// Its declaration, and the wires out of it its file wrote (an
        /// adopted block's), as vislox writes them.
        source: String,
        /// Its program (#79), when its declaration names one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program: Option<String>,
        /// The wires drawn here (#61) that went with it; an empty UUID is
        /// this block.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        wires: Vec<Ends>,
        /// vislox's externs its source names: object UUID → type.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        externs: BTreeMap<String, String>,
        /// Its lockfile entry (`objects.<slug>`).
        lock: serde_json::Value,
        /// The `extern_wires` entries out of its ports.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        lock_wires: Vec<serde_json::Value>,
    },
    /// Rename a block of vislox's files (#80): its slug becomes `slug` in its
    /// declaration and everything vislox's files name it by, and `moved <old>
    /// -> <new>` tells lox to keep its UUIDs. The config does not change: a
    /// block without a name keeps the label its old slug gave it. Its
    /// `previous` is the `rename` back. In a request of its own.
    Rename { uuid: String, slug: String },
    /// Drop the `removed` and `moved` lines lox needs no more (#80, lox's
    /// `stale-lifecycle`). Every edit does this first; this asks for nothing
    /// else. Its `previous` is a `tidy` too.
    Tidy,
    /// Move a block the module manages (#60): its top left corner to
    /// (`x`, `y`) on its page, snapped to Loxone Config's grid. lox keeps a
    /// managed block's position in the lockfile, not in the source, so this
    /// is written there ([`write_moves`]). Its `previous` is the `move` back.
    Move { uuid: String, x: i64, y: i64 },
    /// Lay out the managed blocks of the page with UUID `page` afresh, as
    /// lox places new blocks (`lox logic compile --relayout`). Its
    /// `previous` is the `place` that puts each block back.
    Relayout { page: String },
    /// Move several blocks the module manages at once, each as `move` does:
    /// the undo of a `relayout` (#62), one op however many blocks the page
    /// has. Its `previous` is the `place` that puts them back.
    Place { moves: Vec<Placed> },
    /// Draw a wire (#61) from the output `from` to the input `to`, each
    /// `[object UUID, connector key]` as the page names a wire's ends. It is
    /// written as `to <- from` in [`FILE`]. Its `previous` is the `unwire`.
    Wire { from: [String; 2], to: [String; 2] },
    /// Take out a wire vislox drew (deployed or not). lox removes only the
    /// wires its source draws; one Loxone Config drew is refused. Its
    /// `previous` is the `wire` that draws it again.
    Unwire { from: [String; 2], to: [String; 2] },
    /// Put blocks Loxone Config drew under the module (#72, `lox logic
    /// adopt`): each declared in vislox's file of its page, with its UUIDs.
    /// The config stays exactly as it is. Its `previous` is the `unadopt`.
    Adopt { blocks: Vec<crate::adopt::Pick> },
    /// Adopt every block of the page with UUID `page` that lox can rebuild
    /// exactly; the reply names those it skipped, and why. Its `previous`
    /// is the `unadopt` of those it adopted.
    AdoptPage { page: String },
    /// Stop managing blocks adopted here: their declarations and lockfile
    /// entries go, and Loxone Config owns them again, as the base has them.
    /// Its `previous` is the `adopt` with the same slugs.
    Unadopt { uuids: Vec<String> },
    /// The switching times of the DayTimer `uuid` (#73): a `schedule` set
    /// and the timer's `.schedule` claim in [`FILE`] ([`crate::sched_edit`]).
    /// `None` drops vislox's claim: the timer is as the base has it. Its
    /// `previous` is the `schedule` op with what the timer claimed before.
    Schedule {
        uuid: String,
        value: Option<crate::sched_edit::Sched>,
    },
    /// The moods of the lighting controller `uuid` (#74): a `moods` set and
    /// the controller's `.moods` claim in [`FILE`] ([`crate::mood_edit`]).
    /// `None` drops vislox's claim: the moods are as the base has them. Its
    /// `previous` is the `moods` op with what the controller claimed before.
    Moods {
        uuid: String,
        value: Option<crate::mood_edit::MoodsVal>,
    },
    /// The members of the central block `uuid` (#75): a `.members` claim in
    /// [`FILE`] ([`crate::member_edit`]), the member UUIDs in order (`[]`
    /// empties the list). `None` drops vislox's claim: the members stay as
    /// last written. Its `previous` is the list vislox claimed before.
    Members {
        uuid: String,
        value: Option<Vec<String>>,
    },
    /// Who may open the NFC Code Touch `uuid` (#76): an `access` set and the
    /// Code Touch's `.access` claim in [`FILE`] ([`crate::access_edit`]).
    /// `None` drops vislox's claim: the rights stay as last written. Its
    /// `previous` is the `access` op with what the Code Touch claimed before.
    Access {
        uuid: String,
        value: Option<crate::access_edit::AccessVal>,
    },
    /// Who gets the push notification `uuid` (#77): a `recipients` set and
    /// the notification's `.recipients` claim in [`FILE`]
    /// ([`crate::recipients_edit`]). `None` drops vislox's claim: the
    /// recipients stay as last written. Its `previous` is the `recipients`
    /// op with what the notification claimed before.
    Recipients {
        uuid: String,
        value: Option<crate::recipients_edit::RecipientsVal>,
    },
    /// The tiles of the Touch Pure Flex `uuid` (#78): a `tiles` set and the
    /// device's `.tiles` claim in [`FILE`] ([`crate::tiles_edit`]). `None`
    /// drops vislox's claim: the tiles keep the programs last written. Its
    /// `previous` is the `tiles` op with what the device claimed before.
    Tiles {
        uuid: String,
        value: Option<crate::tiles_edit::TilesVal>,
    },
    /// A `let` constant (#81). One of vislox's file: `let <name> = <value>`
    /// in [`FILE`]; a new name makes it, `None` drops it once nothing is
    /// bound to it, and its `previous` is the value before (none: it did not
    /// exist). One the module's own files declare: its value rewritten on its
    /// line and nothing else ([`crate::module_edit`]), `source` the line as
    /// the browser saw it; its `previous` is the `let` op that writes the old
    /// text back. Every port bound to it takes the new value (lox scales a
    /// suffixed one into each port's unit).
    Let {
        name: String,
        value: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    /// The default of port `port` (a connector key) for every managed block
    /// of `type` that does not bind it (#81): `defaults <Type>(Port: value)`.
    /// On the module's own statement, that value rewritten (`source`: the
    /// statement as the browser saw it); else vislox's statement in [`FILE`],
    /// made when there is none, `None` dropping the value. `blocks` names
    /// exactly the blocks it changes (their slugs), the user's consent. Its
    /// `previous` is the `defaults` op that undoes it.
    Defaults {
        #[serde(rename = "type")]
        ty: String,
        port: String,
        value: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocks: Option<Vec<String>>,
    },
    /// What vislox's files feed into the input `port` (a connector key) of
    /// `uuid` (#82): an expression (`vx_a.Q and vx_b.AQ >= 28`), written
    /// `x.Port <- …` on a block Loxone Config drew and `Port: …` in the
    /// declaration of a block of vislox's files; or a port of vislox's
    /// (`vx_warm.Q`, what *Name this expression* leaves), written as a wire.
    /// `None` drops it. A value op: its `previous` is what fed it before.
    Expr {
        uuid: String,
        port: String,
        value: Option<String>,
    },
    /// A named signal of vislox's page file of `page` (#82): `<slug> =
    /// <value>`, and its label (`<slug>.title`), none showing the
    /// expression. A new slug makes it; `value: None` drops it (not once it
    /// is deployed). Its `previous` is the `signal` op that puts back what was.
    Signal {
        slug: String,
        page: String,
        value: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// A template instance of vislox's page file of `page` (#83): `<slug> =
    /// <Template>("label", param: arg, …)`. A new slug makes it; `value:
    /// None` drops it (not once it is deployed). Its `previous` is the
    /// `instance` op that puts back what was.
    Instance {
        slug: String,
        page: String,
        value: Option<crate::template_edit::Call>,
    },
    /// The statistics of the Tree or Air input or output `uuid` (#85):
    /// `on` or `off`, a hardware declaration with `Statistics:` in vislox's
    /// hardware file ([`crate::hw_edit`]). `None` drops vislox's claim: the
    /// I/O records as the base has it. A value op.
    Statistics { uuid: String, value: Option<String> },
    /// The operating mode a calendar entry of vislox's files switches on
    /// (#86), by the mode's UUID: `switches:` in its declaration, spelled as
    /// lox takes it ([`crate::calendar_edit`]). `None` drops it (lox then
    /// switches the holiday mode). A value op.
    EntryMode { uuid: String, value: Option<String> },
    /// The date of a calendar entry of vislox's files (#86), as the source
    /// writes it: `every: 12-31`, `easter: 1`, `from: 2026-07-04, to:
    /// 2026-09-05` or `from: 10-01, to: 04-30`. `None` drops it (lox then
    /// writes 1 January). A value op.
    EntryDate { uuid: String, value: Option<String> },
    /// What the Automatic Designer rule `uuid` does (#87): a `rule`
    /// declaration in [`FILE`] ([`crate::rule_edit`]): `active`, its
    /// conditions and its actions, blocks and modes by UUID. `None` drops
    /// vislox's declaration: the rule stays as last written. Its `previous`
    /// is what the file stated before (null: nothing).
    Rule {
        uuid: String,
        value: Option<crate::rules::RuleVal>,
    },
    /// A rule named `title` (#87): `rule vx_rule_<n> "Title"` in [`FILE`],
    /// which the compile creates below the config's last rule and gives
    /// `value` (a condition and an action at least). The reply names it (`added`); `remove_block` of
    /// its UUID undoes it.
    NewRule {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<crate::rules::RuleVal>,
        /// The UUID of the rule this creates again: set in the `previous` of
        /// a `remove_block`, as for `add_block`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        restores: Option<String>,
    },
    /// A new logic page (#88): a file of vislox's headed `page "title" (room:
    /// …, category: …)`, from which the compile creates the page. `room` and
    /// `category` are titles of the base's. The reply names its UUID
    /// (`added`); `remove_page` of it undoes it.
    NewPage {
        title: String,
        room: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category: Option<String>,
        /// The UUID of the page this creates again: set in the `previous` of
        /// a `remove_page`, as for `add_block`. lox mints a new one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        restores: Option<String>,
    },
    /// Take out a page `new_page` created that is not deployed yet and holds
    /// nothing of vislox's (#88): its file goes, and the compile no longer
    /// creates it. Its `previous` is the `new_page` that creates it again.
    RemovePage { page: String },
    /// Bring back a rule created here that `remove_block` deleted, while lox
    /// keeps its tombstone: its declaration and the lock's pin, so it keeps
    /// its UUID. Only the `previous` of a `remove_block` makes one; its
    /// `previous` is the `remove_block`.
    RestoreRule {
        uuid: String,
        slug: String,
        title: String,
        value: crate::rules::RuleVal,
    },
}

/// Where a `place` puts one block: its top left corner, as `move` takes it.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Placed {
    pub uuid: String,
    pub x: i64,
    pub y: i64,
}

impl Op {
    /// The object it changes; `None` for a new block.
    pub fn uuid(&self) -> Option<&str> {
        match self {
            Op::Title { uuid, .. }
            | Op::Room { uuid, .. }
            | Op::Category { uuid, .. }
            | Op::Param { uuid, .. }
            | Op::Setting { uuid, .. }
            | Op::RemoveBlock { uuid }
            | Op::RestoreBlock { uuid, .. }
            | Op::Rename { uuid, .. }
            | Op::Move { uuid, .. }
            | Op::Schedule { uuid, .. }
            | Op::Moods { uuid, .. }
            | Op::Members { uuid, .. }
            | Op::Access { uuid, .. }
            | Op::Recipients { uuid, .. }
            | Op::Expr { uuid, .. }
            | Op::Statistics { uuid, .. }
            | Op::EntryMode { uuid, .. }
            | Op::EntryDate { uuid, .. }
            | Op::Rule { uuid, .. }
            | Op::RestoreRule { uuid, .. }
            | Op::Tiles { uuid, .. } => Some(uuid),
            Op::NewRule { .. }
            | Op::NewPage { .. }
            | Op::RemovePage { .. }
            | Op::AddBlock { .. }
            | Op::Signal { .. }
            | Op::Instance { .. }
            | Op::Relayout { .. }
            | Op::Place { .. }
            | Op::Wire { .. }
            | Op::Unwire { .. }
            | Op::Adopt { .. }
            | Op::AdoptPage { .. }
            | Op::Unadopt { .. }
            | Op::Let { .. }
            | Op::Defaults { .. }
            | Op::Tidy => None,
        }
    }

    /// The value it sets (`None` also for a block op or a move).
    pub fn value(&self) -> Option<&str> {
        match self {
            Op::Title { value, .. }
            | Op::Room { value, .. }
            | Op::Category { value, .. }
            | Op::Param { value, .. }
            | Op::Setting { value, .. }
            | Op::Expr { value, .. }
            | Op::Statistics { value, .. }
            | Op::EntryMode { value, .. }
            | Op::EntryDate { value, .. }
            | Op::Let { value, .. } => value.as_deref(),
            Op::AddBlock { .. }
            | Op::Signal { .. }
            | Op::Instance { .. }
            | Op::RemoveBlock { .. }
            | Op::RestoreBlock { .. }
            | Op::Rename { .. }
            | Op::Defaults { .. }
            | Op::Tidy
            | Op::Move { .. }
            | Op::Relayout { .. }
            | Op::Place { .. }
            | Op::Wire { .. }
            | Op::Unwire { .. }
            | Op::Adopt { .. }
            | Op::AdoptPage { .. }
            | Op::Unadopt { .. }
            | Op::Schedule { .. }
            | Op::Moods { .. }
            | Op::Members { .. }
            | Op::Access { .. }
            | Op::Recipients { .. }
            | Op::Rule { .. }
            | Op::NewRule { .. }
            | Op::NewPage { .. }
            | Op::RemovePage { .. }
            | Op::RestoreRule { .. }
            | Op::Tiles { .. } => None,
        }
    }

    /// The same op with another value (a block op stays as it is).
    pub fn with_value(&self, v: Option<String>) -> Op {
        let mut op = self.clone();
        match &mut op {
            Op::Title { value, .. }
            | Op::Room { value, .. }
            | Op::Category { value, .. }
            | Op::Param { value, .. }
            | Op::Setting { value, .. }
            | Op::Expr { value, .. }
            | Op::Statistics { value, .. }
            | Op::EntryMode { value, .. }
            | Op::EntryDate { value, .. }
            | Op::Let { value, .. } => *value = v,
            Op::AddBlock { .. }
            | Op::Signal { .. }
            | Op::Instance { .. }
            | Op::RemoveBlock { .. }
            | Op::RestoreBlock { .. }
            | Op::Rename { .. }
            | Op::Defaults { .. }
            | Op::Tidy
            | Op::Move { .. }
            | Op::Relayout { .. }
            | Op::Place { .. }
            | Op::Wire { .. }
            | Op::Unwire { .. }
            | Op::Adopt { .. }
            | Op::AdoptPage { .. }
            | Op::Unadopt { .. }
            | Op::Schedule { .. }
            | Op::Moods { .. }
            | Op::Members { .. }
            | Op::Access { .. }
            | Op::Recipients { .. }
            | Op::Rule { .. }
            | Op::NewRule { .. }
            | Op::NewPage { .. }
            | Op::RemovePage { .. }
            | Op::RestoreRule { .. }
            | Op::Tiles { .. } => {}
        }
        op
    }
}

/// An attribute of the object itself that vislox claims (`x.title`, `x.room`,
/// `x.category`), in the order the file lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Which {
    Title,
    Room,
    Category,
}

impl Which {
    fn word(self) -> &'static str {
        match self {
            Which::Title => "title",
            Which::Room => "room",
            Which::Category => "category",
        }
    }

    fn op(self, uuid: String, value: Option<String>) -> Op {
        match self {
            Which::Title => Op::Title { uuid, value },
            Which::Room => Op::Room { uuid, value },
            Which::Category => Op::Category { uuid, value },
        }
    }
}

/// `POST /api/edit`: the ops, and the revision they were made against.
#[derive(Deserialize, Debug)]
pub struct Request {
    /// The blob id of the config the browser showed: the pending compile, or
    /// the deployed config when nothing is pending.
    pub base: String,
    pub ops: Vec<Op>,
}

/// The most ops one request takes.
pub const MAX_OPS: usize = 200;

// ── what can be edited ──────────────────────────────────────────────────

/// What Properties may edit on an object. Only the object itself decides
/// (its type, attributes and connectors); the project adds one rule, that
/// blocks the module manages are edited in their own source.
#[derive(Serialize, Debug, Default, PartialEq)]
pub struct Caps {
    pub title: bool,
    /// It has a room, or a category (lox changes them, never adds them).
    pub room: bool,
    pub category: bool,
    pub params: Vec<ParamCap>,
    pub settings: Vec<SettingCap>,
    /// A Tree or Air input or output whose statistics Lox Logic switches
    /// (#85): `on`, `off`, or the other kind Loxone Config set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statistics: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ParamCap {
    /// The connector key (`K=`).
    pub key: String,
    /// The documented unit, as a short suffix (`s`, `%`).
    pub unit: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct SettingCap {
    pub name: String,
    /// Text (a name, a unit); else a number.
    pub text: bool,
    pub value: Option<String>,
}

/// Physical inputs and outputs whose settings Lox Logic can claim, and the
/// settings: (name, text, may be added where the object has none). From the
/// Lox Logic language spec, "`=` on a setting".
const IO_TYPES: &[&str] = &[
    "LoxAIRsensor",
    "LoxAIRAsensor",
    "LoxAIRactor",
    "LoxAIRAactor",
    "TreeSensor",
    "TreeAsensor",
    "TreeActor",
    "TreeAactor",
    "Actor",
    "DigitalIn",
    "VoltageIn",
    "Lox1wireAsensor",
];
const IO_SETTINGS: &[(&str, bool, bool)] = &[
    ("Desc", true, true),
    ("Unit", true, false),
    ("SourceValLow", false, true),
    ("DestValLow", false, true),
    ("SourceValHigh", false, false),
    ("DestValHigh", false, false),
    ("MinVal", false, true),
    ("MaxVal", false, false),
    ("MinChange", false, false),
    ("MinTime", false, false),
    ("ResetTime", false, true),
    ("AverageTime", false, true),
    ("MaxTime", false, false),
];
const DEVICE_TYPES: &[&str] = &["LoxAIRDevice", "TreeDevice"];
const EXTENSION_TYPES: &[&str] = &["LoxTree", "LoxAIR"];
const PLACE_SETTINGS: &[(&str, bool, bool)] = &[
    ("Installation", true, false),
    ("SwitchBoard", true, false),
    ("SwitchBoardRow", false, false),
    ("SwitchBoardPos", false, false),
];
/// Settings that are a duration, a count or a threshold: never below 0.
const NON_NEGATIVE: &[&str] = &[
    "MinChange",
    "MinTime",
    "ResetTime",
    "AverageTime",
    "MaxTime",
    "SwitchBoardRow",
    "SwitchBoardPos",
];

/// The settings `ty` takes: (name, text, may be added).
fn settings_of(ty: &str) -> Vec<(&'static str, bool, bool)> {
    if IO_TYPES.contains(&ty) {
        IO_SETTINGS.to_vec()
    } else if DEVICE_TYPES.contains(&ty) {
        let mut v = vec![("Desc", true, true)];
        v.extend_from_slice(PLACE_SETTINGS);
        v
    } else if EXTENSION_TYPES.contains(&ty) {
        PLACE_SETTINGS.to_vec()
    } else {
        Vec::new()
    }
}

/// Objects that are not edited from Properties at all: folders and pages,
/// and references, whose title Loxone Config takes from what they mirror.
fn not_editable(ty: &str) -> bool {
    matches!(
        ty,
        "Document" | "Page" | "InputRef" | "OutputRef" | "OutputRefLM"
    ) || ty.ends_with("Caption")
}

pub fn unit_suffix(u: PortUnit) -> String {
    match u {
        PortUnit::Milliseconds => "ms".into(),
        PortUnit::Seconds => "s".into(),
        PortUnit::Minutes => "min".into(),
        PortUnit::Percent => "%".into(),
        PortUnit::Kelvin => "K".into(),
        PortUnit::Degrees => "°".into(),
        PortUnit::Other(u) => u.into(),
    }
}

/// What Properties may edit on `el`, or `None` when nothing.
pub fn caps(el: &Element) -> Option<Caps> {
    let ty = el.attr("Type")?;
    el.attr("U")?;
    if not_editable(ty) {
        return None;
    }
    let specs = connectors::builtin(ty);
    let mut params = Vec::new();
    for co in el.child_elements().filter(|c| c.name == "Co") {
        let Some(key) = co.attr("K") else { continue };
        // Wired: the input takes its value from the wire. Inverted: the
        // connector belongs to Loxone Config (lox refuses to set it).
        if co.child_elements().any(|c| c.name == "In") || co.attr("Inv").is_some() {
            continue;
        }
        let input = match specs {
            Some(specs) => specs
                .iter()
                .find(|s| s.key == key)
                .is_some_and(|s| matches!(s.dir, PortDir::Input | PortDir::Param)),
            // A type lox has no port table for: an input that holds a
            // constant is one; an output never does.
            None => co.attr("Def").is_some(),
        };
        if input {
            params.push(ParamCap {
                key: key.to_string(),
                unit: connectors::port_unit(ty, key).map(unit_suffix),
            });
        }
    }
    let settings = settings_of(ty)
        .into_iter()
        .filter(|(name, _, add)| *add || el.attr(name).is_some())
        .map(|(name, text, _)| SettingCap {
            name: name.into(),
            text,
            value: el.attr_decoded(name).map(|v| v.into_owned()),
        })
        .collect();
    let io = el.child_elements().find(|c| c.name == "IoData");
    let bound = |a: &str| io.and_then(|io| io.attr(a)).is_some_and(|v| !v.is_empty());
    let caps = Caps {
        // A rule's name is the app's (its payload holds it too, which a
        // title claim would not change, #87); a rule created here takes its
        // name in its declaration.
        title: el.attr("Title").is_some() && ty != crate::rules::RULE,
        room: bound("Pr"),
        category: bound("Cr"),
        params,
        settings,
        statistics: crate::hw_edit::stats_value(ty).and_then(|_| crate::hw_edit::stats_state(el)),
    };
    (caps.title
        || caps.room
        || caps.category
        || !caps.params.is_empty()
        || !caps.settings.is_empty()
        || caps.statistics.is_some())
    .then_some(caps)
}

// ── the claims file ─────────────────────────────────────────────────────

/// What [`FILE`] claims, by object UUID, and the wires it draws (#61).
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Claims(
    pub BTreeMap<String, Claim>,
    pub BTreeSet<Wire>,
    /// The switching times it writes (#73).
    pub crate::sched_edit::Scheds,
    /// The lighting moods it writes (#74).
    pub crate::mood_edit::MoodSets,
    /// The members of central blocks it claims (#75).
    pub crate::member_edit::Members,
    /// Who may open the NFC Code Touches it claims (#76).
    pub crate::access_edit::AccessSets,
    /// Who gets the push notifications it claims (#77).
    pub crate::recipients_edit::RecipientSets,
    /// The tiles of the Touch Pure Flex devices it claims (#78).
    pub crate::tiles_edit::TileSets,
    /// The `let` constants made here (#81): name → value.
    pub BTreeMap<String, Value>,
    /// The Automatic Designer rules it declares (#87).
    pub crate::rule_edit::Rules,
    /// The `defaults` statements made here (#81): type → port (its Lox
    /// Logic name) → value.
    pub BTreeMap<String, BTreeMap<String, Value>>,
);

/// A wire vislox draws: `to.port <- from.port`, each end a slug (an
/// extern of [`FILE`] or a block vislox added) and a Lox Logic port name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Wire {
    pub to: (String, String),
    pub from: (String, String),
}

impl Wire {
    fn render(&self) -> String {
        format!(
            "{}.{} <- {}.{}",
            self.to.0, self.to.1, self.from.0, self.from.1
        )
    }

    fn touches(&self, slug: &str) -> bool {
        self.to.0 == slug || self.from.0 == slug
    }
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Claim {
    /// The object's type, as the extern declares it.
    pub ty: String,
    /// Its title, room and category.
    pub attrs: BTreeMap<Which, String>,
    /// Port (by its Lox Logic name) or setting → value.
    pub sets: BTreeMap<String, Value>,
    /// Port (by its Lox Logic name) → the expression vislox's file feeds it
    /// (#82), as lox prints it: `x.Port <- …`.
    pub exprs: BTreeMap<String, crate::expr_edit::ExprSrc>,
    /// A block adopted here (#72) names this extern: it stays while it does.
    pub pinned: bool,
}

impl Claim {
    fn is_empty(&self) -> bool {
        self.attrs.is_empty() && self.sets.is_empty() && self.exprs.is_empty() && !self.pinned
    }

    /// It claims a value (not only an extern something names).
    pub fn claims(&self) -> bool {
        !self.attrs.is_empty() || !self.sets.is_empty() || !self.exprs.is_empty()
    }
}

/// The key of vislox's extern of the object `uuid` in [`Claims`]: the one
/// the file has already, case aside (two keys of one UUID would be two
/// `extern vx_…` lines), else `uuid` lowercase.
pub fn extern_key(files: &Files, uuid: &str) -> String {
    files
        .claims
        .0
        .keys()
        .find(|k| k.eq_ignore_ascii_case(uuid))
        .cloned()
        .unwrap_or_else(|| uuid.to_ascii_lowercase())
}

/// The slug vislox gives the object `uuid`.
pub fn slug(uuid: &str) -> String {
    format!(
        "{SLUG_PREFIX}{}",
        uuid.to_ascii_lowercase().replace('-', "_")
    )
}

/// The UUID a slug of [`slug`] names (`vx_6000…_0001` → `6000…-0001`),
/// lowercase.
pub fn slug_uuid(s: &str) -> Option<String> {
    let u = s.strip_prefix(SLUG_PREFIX)?.replace('_', "-");
    // Loxone's UUIDs are 8-4-4-16 hex digits.
    let groups: Vec<usize> = u.split('-').map(str::len).collect();
    (groups == [8, 4, 4, 16] && u.bytes().all(|b| b == b'-' || b.is_ascii_hexdigit())).then_some(u)
}

/// A value as Properties shows it: a number or text, unquoted.
pub fn plain(v: &Value) -> String {
    match v {
        Value::Number(n) => n.clone(),
        Value::Str(s) => s.clone(),
        // A program (#79): its text, as Properties edits it.
        Value::File { text: Some(t), .. } => t.clone(),
        other => other.to_string(),
    }
}

impl Claims {
    /// Read [`FILE`]. Only what vislox writes is accepted, so that a
    /// rewrite never drops a statement someone added by hand.
    pub fn parse(text: &str) -> Result<Claims, String> {
        // A fragment: a wire may name a block of another file of vislox's.
        let m = Module::parse_fragment(text).map_err(|e| e.to_string())?;
        let foreign = |what: &str| {
            format!(
                "it holds {what}, which vislox did not write; move it to another file of the module"
            )
        };
        let mut by_slug: BTreeMap<String, String> = BTreeMap::new();
        let mut c = Claims::default();
        let set = |c: &mut Claims,
                   by_slug: &BTreeMap<String, String>,
                   slug: &str,
                   port: &str,
                   v: Value|
         -> Result<(), String> {
            let uuid = by_slug
                .get(slug)
                .ok_or_else(|| foreign(&format!("`{slug}.{port}`")))?;
            let claim = c.0.get_mut(uuid).expect("declared");
            claim.sets.insert(port.to_string(), v);
            Ok(())
        };
        for item in m.items {
            match item {
                Item::Extern(e) => {
                    let MatchSpec::Uuid(uuid) = &e.match_spec else {
                        return Err(foreign(&format!("`extern {}` not pinned by uuid", e.slug)));
                    };
                    if e.slug != slug(uuid) {
                        return Err(foreign(&format!("`extern {}`", e.slug)));
                    }
                    by_slug.insert(e.slug.clone(), uuid.clone());
                    c.0.insert(
                        uuid.clone(),
                        Claim {
                            ty: e.block_type.clone(),
                            ..Default::default()
                        },
                    );
                    // `lox logic fmt` may fold the assignments into the declaration.
                    for arg in e.args {
                        match arg {
                            ArgItem::Binding(b) => match b.kind {
                                BindingKind::Param(v) => {
                                    set(&mut c, &by_slug, &e.slug, &b.port, v)?
                                }
                                // `lox logic fmt` may fold a wire in too.
                                BindingKind::Wire(r) if !r.port.is_empty() => {
                                    c.1.insert(Wire {
                                        to: (e.slug.clone(), b.port.clone()),
                                        from: (r.slug, r.port),
                                    });
                                }
                                // An expression (#82), folded in by `lox logic fmt`.
                                BindingKind::Expr(x) => {
                                    let uuid = &by_slug[&e.slug];
                                    c.0.get_mut(uuid)
                                        .expect("declared")
                                        .exprs
                                        .insert(b.port.clone(), crate::expr_edit::ExprSrc::of(&x));
                                }
                                _ => {
                                    return Err(foreign(&format!(
                                        "an expression onto `{}`",
                                        e.slug
                                    )));
                                }
                            },
                            _ => return Err(foreign(&format!("`{}`'s arguments", e.slug))),
                        }
                    }
                }
                Item::Set(s) => set(&mut c, &by_slug, &s.target.slug, &s.target.port, s.value)?,
                // An expression onto an extern's input (#82).
                Item::ExprWire(w) => {
                    let uuid = by_slug
                        .get(&w.to.slug)
                        .ok_or_else(|| foreign(&format!("`{} <- {}`", w.to, w.expr)))?;
                    c.0.get_mut(uuid)
                        .expect("declared")
                        .exprs
                        .insert(w.to.port, crate::expr_edit::ExprSrc::of(&w.expr));
                }
                Item::Wire(w) if !w.from.port.is_empty() && !w.to.port.is_empty() => {
                    c.1.insert(Wire {
                        to: (w.to.slug, w.to.port),
                        from: (w.from.slug, w.from.port),
                    });
                }
                Item::Attr(a) => {
                    let which = match a.attr {
                        Attr::Title => Which::Title,
                        Attr::Room => Which::Room,
                        Attr::Category => Which::Category,
                    };
                    let uuid = by_slug
                        .get(&a.slug)
                        .ok_or_else(|| foreign(&format!("`{a}`")))?;
                    c.0.get_mut(uuid)
                        .expect("declared")
                        .attrs
                        .insert(which, plain(&a.value));
                }
                // A constant made here (#81): `vx_…`, a value, never an alias.
                Item::Let(l) => {
                    let value = matches!(
                        l.value,
                        Value::Number(_) | Value::Unit { .. } | Value::Clock(_) | Value::Bool(_)
                    );
                    if !own_constant(&l.name) || !value {
                        return Err(foreign(&format!("`let {} = {}`", l.name, l.value)));
                    }
                    c.8.insert(l.name, l.value);
                }
                // A default made here (#81): values only, one statement a type.
                Item::Defaults(d) => {
                    if c.10.contains_key(&d.block_type) {
                        return Err(foreign(&format!("a second `defaults {}`", d.block_type)));
                    }
                    let mut m = BTreeMap::new();
                    for (port, v) in d.params() {
                        m.insert(port.to_string(), v.clone());
                    }
                    c.10.insert(d.block_type.clone(), m);
                }
                Item::Gap | Item::FragmentEnd | Item::Comment(_) => {}
                // Switching times, moods, members, access: each kind reads
                // its own sets and claims.
                other => match crate::set_claim::KINDS
                    .iter()
                    .find_map(|k| k.read(&mut c, &other))
                {
                    Some(r) => r.map_err(|e| foreign(&e))?,
                    None => return Err(foreign("a statement")),
                },
            }
        }
        // A claim names one of the file's sets, and only vislox's blocks.
        for k in crate::set_claim::KINDS {
            k.check_read(&c).map_err(|e| foreign(&e))?;
        }
        // A wire's ends are vislox's: its externs, or the blocks it added.
        for w in &c.1 {
            for end in [&w.to.0, &w.from.0] {
                if !end.starts_with(SLUG_PREFIX) {
                    return Err(foreign(&format!("the wire `{}`", w.render())));
                }
            }
        }
        Ok(c)
    }

    /// Whether a set claim of the file (switching times, moods) or a member
    /// list (as the central block, or a member) names `slug`.
    pub fn set_claimed(&self, slug: &str) -> bool {
        self.claim_on(slug).is_some()
    }

    /// What a set or member claim of the file claims on `slug`, said after
    /// "vislox's file claims ", and its pronoun.
    pub fn claim_on(&self, slug: &str) -> Option<(String, &'static str)> {
        crate::set_claim::KINDS
            .iter()
            .find_map(|k| k.on(self, slug))
    }

    /// Every slug a wire ends at, or a set or member claim names: read once
    /// per prune or render, not once per extern.
    fn named_slugs(&self) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = self
            .1
            .iter()
            .flat_map(|w| [w.to.0.clone(), w.from.0.clone()])
            .collect();
        for k in crate::set_claim::KINDS {
            out.extend(k.mentions(self));
        }
        // What an expression names (#82): its extern stays while it does.
        for c in self.0.values() {
            for e in c.exprs.values() {
                out.extend(e.reads(&self.8).cloned());
            }
        }
        out
    }

    /// Drop the externs that claim nothing and no wire or set claim names.
    pub(crate) fn prune(&mut self) {
        let named = self.named_slugs();
        let keep: BTreeSet<String> = self
            .0
            .keys()
            .filter(|u| named.contains(&slug(u)))
            .cloned()
            .collect();
        self.0.retain(|u, c| !c.is_empty() || keep.contains(u));
    }

    /// The file's text: a header, then each object's extern and its values.
    pub fn render(&self) -> String {
        let mut out = String::from(
            "# Values edited in vislox (Properties). vislox rewrites this file on every edit;\n\
             # see docs/editing.md in vislox. Deleting a line hands that value back to Loxone Config.\n",
        );
        if !self.8.is_empty() {
            out.push_str(
                "\n# Constants made in vislox. Every port bound to one takes its value.\n",
            );
        }
        for (n, v) in &self.8 {
            out.push_str(&format!("let {n} = {v}\n"));
        }
        if !self.10.is_empty() {
            out.push_str(
                "\n# Defaults made in vislox. Every managed block of the type that does not bind the\n\
                 # port itself takes the value, the module's own blocks among them.\n",
            );
        }
        for (ty, m) in &self.10 {
            let args: Vec<String> = m.iter().map(|(p, v)| format!("{p}: {v}")).collect();
            out.push_str(&format!("defaults {ty}({})\n", args.join(", ")));
        }
        let named = self.named_slugs();
        for (uuid, c) in &self.0 {
            let s = slug(uuid);
            if c.is_empty() && !named.contains(&s) {
                continue;
            }
            out.push_str(&format!(
                "\nextern {s} = {}(uuid: {})\n",
                c.ty,
                Value::Str(uuid.clone())
            ));
            for (w, t) in &c.attrs {
                out.push_str(&format!("{s}.{} = {}\n", w.word(), Value::Str(t.clone())));
            }
            for (k, v) in &c.sets {
                out.push_str(&format!("{s}.{k} = {v}\n"));
            }
            for (k, e) in &c.exprs {
                out.push_str(&format!("{s}.{k} <- {e}\n"));
            }
        }
        for k in crate::set_claim::KINDS {
            out.push_str(&k.render(self));
        }
        if !self.1.is_empty() {
            out.push_str("\n# Wires drawn in vislox. Deleting a line takes the wire out again.\n");
        }
        for w in &self.1 {
            out.push_str(&w.render());
            out.push('\n');
        }
        out
    }

    /// The value claimed for what `op` changes.
    pub fn get(&self, ty: &str, op: &Op) -> Option<String> {
        let c = self.0.get(op.uuid()?)?;
        match op {
            Op::Title { .. } => c.attrs.get(&Which::Title).cloned(),
            Op::Room { .. } => c.attrs.get(&Which::Room).cloned(),
            Op::Category { .. } => c.attrs.get(&Which::Category).cloned(),
            Op::Param { port, .. } => c.sets.get(port_names::name_of(ty, port)).map(plain),
            Op::Setting { name, .. } => c.sets.get(name).map(plain),
            Op::Expr { port, .. } => c
                .exprs
                .get(port_names::name_of(ty, port))
                .map(|e| e.text.clone()),
            Op::AddBlock { .. }
            | Op::Signal { .. }
            | Op::Instance { .. }
            | Op::RemoveBlock { .. }
            | Op::RestoreBlock { .. }
            | Op::Rename { .. }
            | Op::Defaults { .. }
            | Op::Tidy
            | Op::Move { .. }
            | Op::Relayout { .. }
            | Op::Place { .. }
            | Op::Wire { .. }
            | Op::Unwire { .. }
            | Op::Adopt { .. }
            | Op::AdoptPage { .. }
            | Op::Unadopt { .. }
            | Op::Schedule { .. }
            | Op::Moods { .. }
            | Op::Members { .. }
            | Op::Access { .. }
            | Op::Recipients { .. }
            | Op::Tiles { .. }
            | Op::Statistics { .. }
            | Op::EntryMode { .. }
            | Op::EntryDate { .. }
            | Op::Rule { .. }
            | Op::NewRule { .. }
            | Op::NewPage { .. }
            | Op::RemovePage { .. }
            | Op::RestoreRule { .. }
            | Op::Let { .. } => None,
        }
    }

    /// Every claimed value, as the ops that set it.
    pub fn ops(&self) -> Vec<Op> {
        let mut v = Vec::new();
        for (uuid, c) in &self.0 {
            for (w, t) in &c.attrs {
                v.push(w.op(uuid.clone(), Some(t.clone())));
            }
            let settings = settings_of(&c.ty);
            for (k, val) in &c.sets {
                let value = Some(plain(val));
                if settings.iter().any(|(n, ..)| n == k) {
                    v.push(Op::Setting {
                        uuid: uuid.clone(),
                        name: k.clone(),
                        value,
                    });
                } else {
                    v.push(Op::Param {
                        uuid: uuid.clone(),
                        port: port_names::key_or_raw(&c.ty, k).to_string(),
                        value,
                    });
                }
            }
        }
        // The expressions it feeds into inputs (#82).
        for (uuid, c) in &self.0 {
            for (k, e) in &c.exprs {
                v.push(Op::Expr {
                    uuid: uuid.clone(),
                    port: port_names::key_or_raw(&c.ty, k).to_string(),
                    value: Some(e.text.clone()),
                });
            }
        }
        // The constants made here (#81), by name.
        for (n, val) in &self.8 {
            v.push(Op::Let {
                name: n.clone(),
                value: Some(val.to_string()),
                source: None,
            });
        }
        v
    }
}

// ── applying an op ──────────────────────────────────────────────────────

/// The element with UUID `uuid`.
pub fn find<'a>(el: &'a Element, uuid: &str) -> Option<&'a Element> {
    if el.attr("U") == Some(uuid) {
        return Some(el);
    }
    el.child_elements().find_map(|c| find(c, uuid))
}

/// The element with UUID `uuid`, case aside (one walk, kept in `tiles`).
pub use crate::tiles::find_ci;

/// How many objects of type `ty` are titled `title`: a room or category is
/// claimed by its title, which must name exactly one.
fn titled(el: &Element, ty: &str, title: &str) -> usize {
    let me = usize::from(
        el.attr("Type") == Some(ty) && el.attr_decoded("Title").is_some_and(|t| t == title),
    );
    me + el
        .child_elements()
        .map(|c| titled(c, ty, title))
        .sum::<usize>()
}

/// Whether `s` is a plain decimal number (`-2.5`), the only spelling lox
/// writes into `Def=` unchanged.
pub(crate) fn number(s: &str) -> Result<String, String> {
    let n = s.trim();
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let body = n.strip_prefix('-').unwrap_or(n);
    let ok = match body.split_once('.') {
        Some((i, f)) => digits(i) && digits(f),
        None => digits(body),
    };
    if ok {
        Ok(n.to_string())
    } else if n.contains(',') {
        Err(format!(
            "`{n}` is not a number: write a decimal point, not a comma"
        ))
    } else {
        Err(format!("`{n}` is not a number (like 12 or -2.5)"))
    }
}

/// A name or other text value: one line, not empty, not too long.
fn text(s: &str, what: &str, empty_ok: bool) -> Result<String, String> {
    if s.chars().any(|c| c.is_control()) {
        return Err(format!("{what} must be one line of text"));
    }
    if !empty_ok && s.trim().is_empty() {
        return Err(format!("{what} cannot be empty"));
    }
    if s.chars().count() > 120 {
        return Err(format!("{what} is longer than 120 characters"));
    }
    Ok(s.to_string())
}

// ── constants (#81) ─────────────────────────────────────────────────────

/// Whether a typed value is a bare name (`vx_nachlauf`, `temp_schwelle`): it
/// may name a constant (#81); a number never does.
pub fn bare_name(v: &str) -> bool {
    let v = v.trim();
    v.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `name` may be one of vislox's constants: `vx_` and a name of its
/// own, not one vislox names an object of Loxone Config's by.
pub(crate) fn own_constant(name: &str) -> bool {
    name.starts_with(SLUG_PREFIX) && name.len() > SLUG_PREFIX.len() && slug_uuid(name).is_none()
}

/// A port's value typed as the name of a `let` constant (#81), vislox's own
/// (`ours`) or one the module declares (`module`): the binding to it. lox
/// checks the constant's value against the port, scaling a unit.
fn constant_ref(
    ours: &BTreeMap<String, Value>,
    module: &BTreeSet<String>,
    v: &str,
) -> Option<Value> {
    let n = v.trim();
    (ours.contains_key(n) || module.contains(n)).then(|| Value::Ref(n.to_string()))
}

/// A constant's value as Lox Logic writes it: a number, one with its unit
/// (`30min`, `70%`), a time of day (`19:00`) or `on`/`off`. lox checks it
/// against each port bound to it.
pub(crate) fn constant_value(v: &str) -> Result<Value, String> {
    let t = v.trim();
    match t {
        "on" => return Ok(Value::Bool(true)),
        "off" => return Ok(Value::Bool(false)),
        _ => {}
    }
    if logic::clock_seconds(t).is_some() {
        return Ok(Value::Clock(t.to_string()));
    }
    if let Ok(n) = number(t) {
        return Ok(Value::Number(n));
    }
    if t.starts_with(|c: char| c.is_ascii_digit() || c == '-')
        && let Some(u @ Value::Unit { .. }) = crate::units::lox_value(t)
    {
        return Ok(u);
    }
    Err(format!(
        "`{t}` is no constant: a number, one with its unit (30min, 70%), a time of day (19:00), \
         or on or off"
    ))
}

/// How many ports vislox's files bind to the constant `name`.
pub fn constant_uses(files: &Files, name: &str) -> usize {
    let is = |v: &Value| matches!(v, Value::Ref(r) if r == name);
    let claims: usize = files
        .claims
        .0
        .values()
        .map(|c| c.sets.values().filter(|v| is(v)).count())
        .sum();
    let blocks: usize = files
        .pages
        .values()
        .flat_map(|p| &p.blocks)
        .map(|b| b.args.iter().filter(|(_, v)| is(v)).count())
        .sum();
    claims + blocks
}

/// The `let` op: make, change or drop one of vislox's constants. Returns the
/// value before (`None`: there was none).
fn set_constant(
    files: &mut Files,
    ctx: &Ctx,
    name: &str,
    value: Option<&str>,
) -> Result<Option<String>, String> {
    let name = name.trim();
    let previous = files.claims.8.get(name).map(|v| v.to_string());
    if previous.is_none() {
        let Some(_) = value else {
            return Err(format!("vislox's file has no constant `{name}`"));
        };
        logic::valid_slug(name).map_err(|e| e.to_string())?;
        if !own_constant(name) {
            return Err(format!(
                "vislox's constants are named {SLUG_PREFIX} and then lowercase letters, digits \
                 and _"
            ));
        }
        if name.contains("__") {
            return Err("`__` is lox's for the blocks an expression makes".into());
        }
        if ctx.names.contains(name)
            || ctx.lock.slugs.contains_key(name)
            || ctx.lock.tombstoned(name)
            || files.names(name)
        {
            return Err(format!(
                "`{name}` is taken: the module or its lockfile has that name"
            ));
        }
    }
    match value {
        Some(v) => {
            let v = constant_value(v)?;
            files.claims.8.insert(name.to_string(), v);
        }
        None => {
            let n = constant_uses(files, name);
            if n > 0 {
                return Err(format!(
                    "`{name}` is still bound to {n} port{}: give them values of their own first",
                    if n == 1 { "" } else { "s" }
                ));
            }
            files.claims.8.remove(name);
        }
    }
    Ok(previous)
}

/// Apply `op` to `claims`, checked against the pending config `doc`.
/// Returns the value claimed before (`None`: none), which undoes it.
pub fn apply(
    claims: &mut Claims,
    doc: &LoxoneDoc,
    managed: &HashSet<String>,
    lets: &BTreeSet<String>,
    op: &Op,
) -> Result<Option<String>, String> {
    let uuid = op
        .uuid()
        .filter(|_| {
            !matches!(
                op,
                Op::RemoveBlock { .. }
                    | Op::Move { .. }
                    | Op::Schedule { .. }
                    | Op::Moods { .. }
                    | Op::Members { .. }
                    | Op::Access { .. }
                    | Op::Recipients { .. }
                    | Op::Tiles { .. }
                    | Op::Statistics { .. }
                    | Op::EntryMode { .. }
                    | Op::EntryDate { .. }
                    | Op::Rule { .. }
                    | Op::NewRule { .. }
                    | Op::NewPage { .. }
                    | Op::RemovePage { .. }
                    | Op::RestoreRule { .. }
            )
        })
        .ok_or_else(|| "not a value".to_string())?;
    let el = find(&doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let ty = el.attr("Type").unwrap_or_default().to_string();
    let name = el.attr_decoded("Title").unwrap_or_default().into_owned();
    if managed.contains(uuid) {
        return Err(format!(
            "{name}: the module manages this block; change it in its .loxl file"
        ));
    }
    let previous = claims.get(&ty, op);
    let caps = caps(el).unwrap_or_default();
    // `None` drops the claim, also where the value can no longer be edited.
    let (key, value): (Result<Which, String>, Option<Value>) = match op {
        Op::Title { value, .. } => {
            let v = match value {
                Some(v) if caps.title => Some(Value::Str(text(v, "The name", false)?)),
                Some(_) => return Err(format!("{name}: the name cannot be edited here")),
                None => None,
            };
            (Ok(Which::Title), v)
        }
        Op::Room { value, .. } | Op::Category { value, .. } => {
            let room = matches!(op, Op::Room { .. });
            let (what, kind, can) = if room {
                ("room", "Place", caps.room)
            } else {
                ("category", "Category", caps.category)
            };
            let v = match value {
                Some(v) if can => {
                    let n = titled(&doc.xml.root, kind, v);
                    if n != 1 {
                        return Err(format!(
                            "{name}: {} {what} is titled \"{v}\"; pick one of the config's {what}s",
                            if n == 0 { "no" } else { "more than one" }
                        ));
                    }
                    Some(Value::Str(v.clone()))
                }
                Some(_) => {
                    return Err(format!(
                        "{name}: it has no {what}; set one in Loxone Config first"
                    ));
                }
                None => None,
            };
            (Ok(if room { Which::Room } else { Which::Category }), v)
        }
        Op::Param { port, value, .. } => {
            let pname = port_names::name_of(&ty, port).to_string();
            let v = match value {
                Some(v) if caps.params.iter().any(|p| &p.key == port) => {
                    Some(match constant_ref(&claims.8, lets, v) {
                        Some(c) => c,
                        None => crate::units::port_value(&ty, port, v)
                            .map_err(|e| format!("{pname}: {e}"))?,
                    })
                }
                Some(_) => {
                    return Err(format!(
                        "{name}: {pname} cannot be edited here (it is wired, or not an input)"
                    ));
                }
                None => None,
            };
            (Err(pname), v)
        }
        Op::Setting {
            name: setting,
            value,
            ..
        } => {
            let v = match (value, caps.settings.iter().find(|s| &s.name == setting)) {
                // Settings are optional text: empty clears them (`Desc=""`).
                (Some(v), Some(cap)) if cap.text => Some(Value::Str(text(v, setting, true)?)),
                (Some(v), Some(_)) => {
                    let n = number(v).map_err(|e| format!("{setting}: {e}"))?;
                    if NON_NEGATIVE.contains(&setting.as_str()) && n.starts_with('-') {
                        return Err(format!("{setting}: cannot be below 0"));
                    }
                    Some(Value::Number(n))
                }
                (Some(_), None) => {
                    return Err(format!("{name}: {setting} cannot be edited here"));
                }
                (None, _) => None,
            };
            (Err(setting.clone()), v)
        }
        Op::AddBlock { .. }
        | Op::RemoveBlock { .. }
        | Op::RestoreBlock { .. }
        | Op::Rename { .. }
        | Op::Defaults { .. }
        | Op::Tidy
        | Op::Move { .. }
        | Op::Relayout { .. }
        | Op::Place { .. }
        | Op::Wire { .. }
        | Op::Unwire { .. }
        | Op::Adopt { .. }
        | Op::AdoptPage { .. }
        | Op::Unadopt { .. }
        | Op::Schedule { .. }
        | Op::Moods { .. }
        | Op::Members { .. }
        | Op::Access { .. }
        | Op::Recipients { .. }
        | Op::Tiles { .. }
        | Op::Expr { .. }
        | Op::Signal { .. }
        | Op::Instance { .. }
        | Op::Statistics { .. }
        | Op::EntryMode { .. }
        | Op::EntryDate { .. }
        | Op::Rule { .. }
        | Op::NewRule { .. }
        | Op::NewPage { .. }
        | Op::RemovePage { .. }
        | Op::RestoreRule { .. }
        | Op::Let { .. } => {
            unreachable!("not a value")
        }
    };
    let claim = claims.0.entry(uuid.to_string()).or_default();
    claim.ty = ty;
    match (key, value) {
        (Ok(w), Some(v)) => {
            claim.attrs.insert(w, plain(&v));
        }
        (Ok(w), None) => {
            claim.attrs.remove(&w);
        }
        (Err(k), Some(v)) => {
            claim.sets.insert(k, v);
        }
        (Err(k), None) => {
            claim.sets.remove(&k);
        }
    }
    claims.prune();
    Ok(previous)
}

// ── vislox's files, and the blocks it adds ──────────────────────────────

/// What the lockfile says about managed blocks: slug → UUID, the slugs of
/// its removal tombstones (not to be picked again), and where the blocks
/// drawn on a page are.
#[derive(Default, Clone, Debug)]
pub struct Lock {
    /// The managed blocks: slug → UUID ([`Lock::read`] derives it from
    /// `entries`).
    pub slugs: BTreeMap<String, String>,
    /// The blocks lox keeps a position (`layout`) for, so drawn on a page
    /// (#60): slug → the page it pins them to (`page_uuid`; `None` in a lock
    /// from before lox pinned pages). Objects of the object tree have none.
    pub placed: BTreeMap<String, Option<String>>,
    /// The wires lox draws onto the ports of externs (`extern_wires`):
    /// (source connector UUID, sink connector UUID).
    pub wires: BTreeSet<(String, String)>,
    /// The objects the module's externs resolved to (`externals`): slug →
    /// UUID. Loxone Config's, not managed; an extern by title (`Notification(
    /// title: "Frost")`) names its object only here (#68).
    pub externals: BTreeMap<String, String>,
    /// Each managed block's entry as the lock has it (`objects.<slug>`): what
    /// restoring a deleted block (#80) puts back, so it keeps its UUIDs. Read
    /// through [`Lock::entry`], which checks it against `slugs`.
    pub entries: BTreeMap<String, serde_json::Value>,
    /// The removal tombstones (`removed`): object UUID → (slug, type). The
    /// one place they are kept; [`Lock::tombstoned`] asks by slug.
    pub tombs: BTreeMap<String, (String, String)>,
    /// The externals a hardware declaration owns (`owned`, #85): the module
    /// writes their title, room, category and settings.
    pub owned: BTreeSet<String>,
    /// The named Automatic Designer rules (`rules`, #87): slug → rule UUID.
    pub rules: BTreeMap<String, String>,
    /// The pages lox created from a `page "…" (room: …)` line (`pages`,
    /// #88): title → page UUID.
    pub pages: BTreeMap<String, String>,
}

impl Lock {
    pub fn read(path: &Path) -> Lock {
        let Ok(bytes) = std::fs::read(path) else {
            return Lock::default();
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return Lock::default();
        };
        let entries: BTreeMap<String, serde_json::Value> = v
            .get("objects")
            .and_then(|o| o.as_object())
            .map(|o| o.iter().map(|(s, x)| (s.clone(), x.clone())).collect())
            .unwrap_or_default();
        let slugs = entries
            .iter()
            .filter_map(|(s, x)| Some((s.clone(), x.get("uuid")?.as_str()?.to_string())))
            .collect();
        let placed = v
            .get("objects")
            .and_then(|o| o.as_object())
            .map(|o| {
                o.iter()
                    .filter(|(_, x)| x.get("layout").is_some_and(|l| l.is_object()))
                    .map(|(s, x)| {
                        let page = x
                            .get("page_uuid")
                            .and_then(|p| p.as_str())
                            .map(String::from);
                        (s.clone(), page)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let wires = v
            .get("extern_wires")
            .and_then(|o| o.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|w| {
                        Some((
                            w.get("from")?.as_str()?.to_string(),
                            w.get("to")?.as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let externals = v
            .get("externals")
            .and_then(|o| o.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(s, x)| Some((s.clone(), x.get("uuid")?.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let tombs = v
            .get("removed")
            .and_then(|o| o.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(u, x)| {
                        let slug = x.get("slug")?.as_str()?.to_string();
                        let ty = x.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        Some((u.clone(), (slug, ty.to_string())))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let owned = v
            .get("externals")
            .and_then(|o| o.as_object())
            .map(|o| {
                o.iter()
                    .filter(|(_, x)| x.get("owned").and_then(|b| b.as_bool()) == Some(true))
                    .map(|(s, _)| s.clone())
                    .collect()
            })
            .unwrap_or_default();
        let rules = v
            .get("rules")
            .and_then(|o| o.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(s, u)| Some((s.clone(), u.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let pages = v
            .get("pages")
            .and_then(|o| o.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(t, u)| Some((t.clone(), u.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        Lock {
            slugs,
            placed,
            wires,
            externals,
            entries,
            tombs,
            owned,
            rules,
            pages,
        }
    }

    /// Whether a removal tombstone has the slug `slug` (not picked again).
    pub fn tombstoned(&self, slug: &str) -> bool {
        self.tombs.values().any(|(s, _)| s == slug)
    }

    /// The lockfile entry of the managed block `slug`, when it is the one
    /// `slugs` names.
    pub fn entry(&self, slug: &str) -> Option<&serde_json::Value> {
        let uuid = self.slugs.get(slug)?;
        self.entries.get(slug).filter(|e| {
            e.get("uuid")
                .and_then(|u| u.as_str())
                .is_some_and(|u| u.eq_ignore_ascii_case(uuid))
        })
    }

    /// The tombstone of the object `uuid` (case aside): its UUID as the lock
    /// spells it, its slug and its type.
    pub fn tomb(&self, uuid: &str) -> Option<(&str, &str, &str)> {
        self.tombs
            .iter()
            .find(|(u, _)| u.eq_ignore_ascii_case(uuid))
            .map(|(u, (s, t))| (u.as_str(), s.as_str(), t.as_str()))
    }

    pub fn slug_of(&self, uuid: &str) -> Option<&str> {
        self.slugs
            .iter()
            .find(|(_, u)| u.eq_ignore_ascii_case(uuid))
            .map(|(s, _)| s.as_str())
    }

    pub fn uuid_of(&self, slug: &str) -> Option<&str> {
        self.slugs.get(slug).map(String::as_str)
    }
}

/// vislox's files in the module ([`DIR`]): the values it claims ([`FILE`])
/// and the blocks it added, by file name.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Files {
    pub claims: Claims,
    pub pages: BTreeMap<String, PageFile>,
    /// The files there were. [`FILE`] stays once there, empty or not; a
    /// page file goes with its last block.
    pub present: BTreeSet<String>,
    /// What an edit moves (#60). Not a file of vislox's: lox keeps a managed
    /// block's position in the lockfile, so it is written there.
    pub layout: Placement,
    /// What adopting a block (#72) or its undo changes in the lockfile.
    pub lock_edits: crate::adopt::LockEdits,
    /// The templates of lox's library vislox copied in for its instances
    /// (#83), by name: [`TEMPLATES`]. One goes with the last instance of
    /// vislox's that calls it.
    pub templates: BTreeMap<String, logic::TemplateDecl>,
    /// The Tree and Air hardware it declares (#85): [`crate::hw_edit::FILE`].
    pub hardware: crate::hw_edit::HwFile,
    /// The module's own files a request rewrites (#81): one `let` or
    /// `defaults` value changed, by path from the project root
    /// ([`crate::module_edit`]). Not read from vislox's directory: an edit
    /// writes them, and Discard takes them back with the rest of the module.
    pub module: BTreeMap<String, String>,
}

/// The positions an edit gives managed blocks, and the pages it lays out
/// afresh. lox has no Lox Logic statement for a position: it keeps the
/// rectangle of each block it manages in the lockfile, and takes a move
/// made in Loxone Config over from the base.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Placement {
    /// Slug → its new rectangle.
    pub moves: BTreeMap<String, Layout>,
    /// The pages to lay out afresh: UUID → the title lox takes (`--relayout`).
    pub relayout: BTreeMap<String, String>,
}

/// No templates: a [`Ctx`] of a request that names none (#83).
#[cfg(test)]
pub(crate) static NO_TEMPLATES: BTreeMap<String, logic::TemplateDecl> = BTreeMap::new();

/// The name of [`FILE`] in [`DIR`].
pub(crate) const CLAIMS: &str = "edits.loxl";

/// The file of the library templates vislox copied in (#83), in [`DIR`].
pub(crate) const TEMPLATES: &str = "templates.loxl";

impl Files {
    /// The files from their texts, by name ([`Files::texts`]). A file
    /// vislox did not write is an error that names it.
    pub fn from_texts(texts: &BTreeMap<String, String>) -> Result<Files, String> {
        let mut f = Files {
            present: texts.keys().cloned().collect(),
            ..Files::default()
        };
        let mut programs = BTreeMap::new();
        // The claims first: a page file's bare name of one of its constants
        // (#81) is a value.
        if let Some(text) = texts.get(CLAIMS) {
            f.claims = Claims::parse(text).map_err(|e| format!("{CLAIMS}: {e}"))?;
        }
        let consts: BTreeSet<String> = f.claims.8.keys().cloned().collect();
        for (name, text) in texts {
            if name.ends_with(PROGRAM_EXT) {
                programs.insert(name.clone(), text.replace("\r\n", "\n"));
            } else if name == TEMPLATES && !crate::template_edit::is_page_file(text) {
                // A page titled "Templates" before #83 reserved the name keeps
                // its file: a page file has a page line, the templates file none.
                f.templates = crate::template_edit::parse_templates(text)
                    .map_err(|e| format!("{name}: {e}"))?;
            } else if name == crate::hw_edit::FILE {
                // Tree and Air hardware (#85): no page line.
                f.hardware =
                    crate::hw_edit::HwFile::parse(text).map_err(|e| format!("{name}: {e}"))?;
            } else if name == crate::periphery::FILE {
                // The objects of the object tree (#84): no page line.
                f.pages.insert(
                    name.clone(),
                    PageFile::parse_periphery(text, &consts).map_err(|e| format!("{name}: {e}"))?,
                );
            } else if name != CLAIMS {
                f.pages.insert(
                    name.clone(),
                    PageFile::parse_with(text, &consts).map_err(|e| format!("{name}: {e}"))?,
                );
            }
        }
        // A program (#79) is in a file of vislox's directory that its block
        // names, `Code: file("vx_code16_1.c")`; each file is one block's: an
        // edit of one block's program must not change another's.
        let mut named: BTreeMap<String, String> = BTreeMap::new();
        for (name, p) in f.pages.iter_mut() {
            for b in &mut p.blocks {
                for (k, v) in &mut b.args {
                    if let Value::File { path, text } = v {
                        if let Some(other) = named.insert(path.clone(), b.slug.clone()) {
                            return Err(format!(
                                "{name}: `{other}` and `{}` both name file(\"{path}\"); a program \
                                 file is one block's, so give each block its own",
                                b.slug
                            ));
                        }
                        let t = programs.get(path.as_str()).ok_or_else(|| {
                            format!(
                                "{name}: `{}`'s {k} names file(\"{path}\"), which is not a program \
                                 in vislox's directory; put it back or delete the setting",
                                b.slug
                            )
                        })?;
                        *text = Some(t.clone());
                    }
                }
            }
        }
        programs.retain(|n, _| !named.contains_key(n));
        if let Some(name) = programs.keys().next() {
            return Err(format!(
                "{name}: no block of vislox's files names this program; delete it, or put back \
                 the block's `Code: file(\"{name}\")`"
            ));
        }
        // A block adopted here names externs of the claims file, or blocks of
        // vislox's files; those externs stay while it does.
        for (s, file) in f.adopted_names() {
            if f.block(&s).is_some() || f.signal(&s).is_some() || f.instance(&s).is_some() {
                continue;
            }
            match f.claims.0.iter_mut().find(|(u, _)| slug(u) == s) {
                Some((_, c)) => c.pinned = true,
                None => {
                    return Err(format!(
                        "{file}: a block adopted there names `{s}`, which is neither an extern of \
                         {CLAIMS} nor a block of vislox's files; put its line back"
                    ));
                }
            }
        }
        // A set claim (#73, #74, #76) or a members claim (#75) names externs
        // of the claims file or blocks of vislox's files.
        for k in crate::set_claim::KINDS {
            for (stmt, t) in k.names(&f.claims) {
                if !f.claims.0.keys().any(|u| slug(u) == t) && f.block(&t).is_none() {
                    return Err(format!(
                        "{CLAIMS}: `{stmt}` names `{t}`, which is neither an extern of this file \
                         nor a block vislox added; put its line back or delete the claim"
                    ));
                }
            }
        }
        // Every end of a wire is an extern of the claims file or a block
        // vislox added; anything else would break every compile.
        for w in &f.claims.1 {
            for (s, _) in [&w.from, &w.to] {
                let ext = f.claims.0.keys().any(|u| slug(u) == *s);
                if !ext && f.block(s).is_none() && f.signal(s).is_none() {
                    return Err(format!(
                        "{CLAIMS}: the wire `{}` names `{s}`, which is neither an extern of this \
                         file nor a block vislox added; put its line back or delete the wire",
                        w.render()
                    ));
                }
            }
        }
        Ok(f)
    }

    /// The `.loxl` files of `dir` and the programs there (#79), by name.
    pub fn texts(dir: &Path) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !ours(&name) || !e.path().is_file() {
                continue;
            }
            if let Ok(t) = std::fs::read_to_string(e.path()) {
                out.insert(name, t);
            }
        }
        out
    }

    /// Each `.loxl` file's text, by name (the programs: [`Files::programs`]). A page file with no block and no
    /// `removed` line is left out (deleted); [`FILE`] only while it was never
    /// written and claims nothing.
    pub fn render(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        if !self.claims.0.values().all(Claim::is_empty)
            || !self.claims.1.is_empty()
            || !self.claims.8.is_empty()
            || !self.claims.10.is_empty()
            || crate::set_claim::KINDS
                .iter()
                .any(|k| !k.is_empty(&self.claims))
            || self.present.contains(CLAIMS)
        {
            out.insert(CLAIMS.to_string(), self.claims.render());
        }
        for (name, p) in &self.pages {
            if !p.is_empty() {
                out.insert(name.clone(), p.render());
            }
        }
        if !self.hardware.is_empty() {
            out.insert(crate::hw_edit::FILE.to_string(), self.hardware.render());
        }
        if !self.templates.is_empty() && !self.pages.contains_key(TEMPLATES) {
            out.insert(
                TEMPLATES.to_string(),
                crate::template_edit::render_templates(&self.templates),
            );
        }
        out
    }

    /// Each program (#79) by the file its block names, borrowed: a program
    /// may be long, and is written only when it changed.
    pub fn programs(&self) -> BTreeMap<&str, &str> {
        let mut out = BTreeMap::new();
        for p in self.pages.values() {
            for b in &p.blocks {
                for (_, v) in &b.args {
                    if let Value::File {
                        path,
                        text: Some(t),
                    } = v
                    {
                        out.insert(path.as_str(), t.as_str());
                    }
                }
            }
        }
        out
    }

    /// Every file's text, the programs among them (tests, and what a scratch
    /// copy needs).
    #[cfg(test)]
    pub fn all_texts(&self) -> BTreeMap<String, String> {
        let mut out = self.render();
        for (n, t) in self.programs() {
            out.insert(n.to_string(), t.to_string());
        }
        out
    }

    /// The text of [`FILE`] with nothing claimed: what keeps a module
    /// directory from having no `.loxl` file at all, which lox refuses.
    pub fn placeholder() -> (String, String) {
        (CLAIMS.to_string(), Claims::default().render())
    }

    /// Tie each page file to its page in `base` by UUID, and follow a page
    /// Loxone Config renamed: `page "…"` names the page by its title. A file
    /// written before it had a UUID takes the first page with its title. A
    /// file that creates its page (`room:`, #88) is left to
    /// [`crate::page_edit::hand_off`], which ties it once the page is deployed.
    pub fn follow_pages(&mut self, base: &LoxoneDoc) {
        for p in self
            .pages
            .values_mut()
            .filter(|p| !p.periphery && p.room.is_none())
        {
            match &p.uuid {
                Some(u) => {
                    if let Some(t) = find_ci(&base.xml.root, u)
                        .filter(|e| e.attr("Type") == Some("Page"))
                        .and_then(|e| e.attr_decoded("Title"))
                        .filter(|t| !t.trim().is_empty())
                    {
                        p.page = t.into_owned();
                    }
                }
                None => {
                    let mut same = Vec::new();
                    pages_titled(&base.xml.root, &p.page, &mut same);
                    p.uuid = same.first().and_then(|e| e.attr("U")).map(String::from);
                }
            }
        }
    }

    /// The names the blocks adopted here use for the objects they are wired
    /// to (not themselves), and the notifications the blocks here mirror,
    /// with the file that names them.
    pub fn adopted_names(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (name, p) in &self.pages {
            // What the template instances here name (#83).
            for x in &p.instances {
                for n in &x.names {
                    out.push((n.clone(), name.clone()));
                }
            }
            for b in p.blocks.iter().filter(|b| b.adopted) {
                for (_, src) in &b.wires {
                    out.push((src.slug.clone(), name.clone()));
                }
            }
            // The notification a push notification's block mirrors (#77),
            // on a block added here too, and the device an I/O hangs below
            // (#84).
            for b in &p.blocks {
                for k in [blocks::MIRRORS, crate::periphery::DEVICE] {
                    if let Some(Value::Ref(n)) = b.get(k) {
                        out.push((n.clone(), name.clone()));
                    }
                }
            }
            for w in &p.wires {
                out.push((w.to.slug.clone(), name.clone()));
            }
            // What the expressions here name (#82): externs of the claims
            // file stay while they do.
            let exprs = p
                .blocks
                .iter()
                .flat_map(|b| b.exprs.iter().map(|(_, e)| e))
                .chain(p.signals.iter().map(|x| &x.expr));
            for e in exprs {
                for n in e.reads(&self.claims.8) {
                    out.push((n.clone(), name.clone()));
                }
            }
        }
        out
    }

    /// Mark the externs the blocks adopted here name, and drop the externs
    /// nothing claims, wires or names any more.
    pub(crate) fn repin(&mut self) {
        let names: BTreeSet<String> = self.adopted_names().into_iter().map(|(s, _)| s).collect();
        for (u, c) in self.claims.0.iter_mut() {
            c.pinned = names.contains(&slug(u));
        }
        self.claims.prune();
    }

    /// The block vislox added as `slug`: its file and declaration.
    pub fn block(&self, slug: &str) -> Option<(&str, &NewBlock)> {
        self.pages.iter().find_map(|(n, p)| {
            p.blocks
                .iter()
                .find(|b| b.slug == slug)
                .map(|b| (n.as_str(), b))
        })
    }

    /// The named signal of vislox's files `slug` (#82): its file and itself.
    pub fn signal(&self, slug: &str) -> Option<(&str, &blocks::SignalDef)> {
        self.pages.iter().find_map(|(n, p)| {
            p.signals
                .iter()
                .find(|x| x.slug == slug)
                .map(|x| (n.as_str(), x))
        })
    }

    pub(crate) fn block_mut(&mut self, slug: &str) -> Option<&mut NewBlock> {
        self.pages
            .values_mut()
            .find_map(|p| p.blocks.iter_mut().find(|b| b.slug == slug))
    }

    /// Whether a block, removal, rename, set or constant (#81) of vislox's files is
    /// named `slug`.
    pub fn names(&self, slug: &str) -> bool {
        self.names_with(slug, true)
    }

    /// [`Files::names`]; a `removed` line counts only with `removed` (#82: a
    /// signal dropped here and made again).
    pub fn names_with(&self, slug: &str, removed: bool) -> bool {
        self.names_except_removed(slug)
            || (removed && self.hardware.removed.contains(slug))
            || self
                .pages
                .values()
                .any(|p| removed && p.removed.contains(slug))
    }

    /// [`Files::names`] without the `removed` lines.
    fn names_except_removed(&self, slug: &str) -> bool {
        (self.hardware.names(slug) && !self.hardware.removed.contains(slug))
            || self.claims.8.contains_key(slug)
            || crate::set_claim::KINDS
                .iter()
                .any(|k| k.set_names(&self.claims).contains(&slug))
            || self.pages.values().any(|p| {
                p.moved.iter().any(|(f, t)| f == slug || t == slug)
                    || p.blocks.iter().any(|b| b.slug == slug)
                    || p.signals.iter().any(|x| x.slug == slug)
                    || p.instances.iter().any(|x| x.slug == slug)
            })
    }

    /// The template instance of vislox's files `slug` (#83): its file and
    /// itself.
    pub fn instance(&self, slug: &str) -> Option<(&str, &blocks::InstanceDef)> {
        self.pages.iter().find_map(|(n, p)| {
            p.instances
                .iter()
                .find(|x| x.slug == slug)
                .map(|x| (n.as_str(), x))
        })
    }

    /// Drop the `removed` and `moved` lines lox needs no more (what `lox
    /// logic lint` calls `stale-lifecycle`): the lockfile has forgotten the
    /// block (or never knew it, when it was added and taken out before a
    /// compile; a deployed one stays deleted by its tombstone), or the old
    /// slug of a rename (lox renamed the entry).
    pub fn tidy(&mut self, lock: &Lock) {
        for p in self.pages.values_mut() {
            p.removed.retain(|s| lock.slugs.contains_key(s));
            let kept = p.removed.clone();
            p.dropped.retain(|s| kept.contains(s));
            p.unstamped.retain(|s| kept.contains(s));
            p.moved.retain(|from, _| lock.slugs.contains_key(from));
        }
        // A created device's (#85): the lockfile keeps it as an extern.
        self.hardware
            .removed
            .retain(|s| lock.externals.contains_key(s));
        // A rule created here (#87).
        crate::rule_edit::tidy(&mut self.claims.9, lock);
    }

    /// The lines [`Files::tidy`] would drop, as written.
    pub fn stale_lines(&self, lock: &Lock) -> Vec<String> {
        let mut out = Vec::new();
        for p in self.pages.values() {
            for s in p.removed.iter().filter(|s| !lock.slugs.contains_key(*s)) {
                out.push(format!("removed {s}"));
            }
            for (f, t) in p.moved.iter().filter(|(f, _)| !lock.slugs.contains_key(*f)) {
                out.push(format!("moved {f} -> {t}"));
            }
        }
        for s in &self.hardware.removed {
            if !lock.externals.contains_key(s) {
                out.push(format!("removed {s}"));
            }
        }
        out.extend(crate::rule_edit::stale_lines(&self.claims.9, lock));
        out
    }

    /// Every value vislox claims or wrote into a block it added (by the
    /// block's UUID in `lock`), as the ops that set it.
    #[cfg(test)]
    pub fn ops(&self, lock: &Lock) -> Vec<Op> {
        self.ops_in(lock, None)
    }

    /// [`Files::ops`], with the mode a calendar entry switches on (#86) by
    /// its UUID in `doc`, which a title names.
    pub fn ops_in(&self, lock: &Lock, doc: Option<&LoxoneDoc>) -> Vec<Op> {
        let modes = doc.map(|d| crate::sched_edit::Modes::new(d, lock));
        let mut v = self.claims.ops();
        v.extend(self.hardware.ops(lock));
        for p in self.pages.values() {
            for b in &p.blocks {
                let Some(uuid) = lock.uuid_of(&b.slug) else {
                    continue;
                };
                if let Some(t) = &b.title {
                    v.push(Op::Title {
                        uuid: uuid.into(),
                        value: Some(t.clone()),
                    });
                }
                let (params, settings) = split_args(b);
                for (port, value) in params {
                    v.push(Op::Param {
                        uuid: uuid.into(),
                        port,
                        value: Some(value),
                    });
                }
                for (name, value) in settings {
                    v.push(Op::Setting {
                        uuid: uuid.into(),
                        name,
                        value: Some(value),
                    });
                }
                // The expressions into its inputs (#82).
                for (port, e) in &b.exprs {
                    v.push(Op::Expr {
                        uuid: uuid.into(),
                        port: blocks::port_key(&b.ty, port).to_string(),
                        value: Some(e.text.clone()),
                    });
                }
                // A calendar entry's mode and date (#86).
                if b.ty == crate::calendar_edit::ENTRY {
                    if let Some(m) = modes
                        .as_ref()
                        .and_then(|m| crate::calendar_edit::mode_uuid(m, &b.args))
                    {
                        v.push(Op::EntryMode {
                            uuid: uuid.into(),
                            value: Some(m),
                        });
                    }
                    if let Some(d) = crate::calendar_edit::date_text(&b.args) {
                        v.push(Op::EntryDate {
                            uuid: uuid.into(),
                            value: Some(d),
                        });
                    }
                }
            }
        }
        v
    }
}

/// A new block's arguments as the ops name them: port key → constant, and
/// setting → value, each as Properties shows it.
pub fn split_args(b: &NewBlock) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let (mut params, mut settings) = (BTreeMap::new(), BTreeMap::new());
    for (k, v) in &b.args {
        // The device an I/O hangs below (#84): where it is, which `add_block`
        // takes as its `page`, never a value.
        if k == crate::periphery::DEVICE {
            continue;
        }
        // A calendar entry's mode and date (#86): values of their own, read
        // with the config (`entry_mode`, `entry_date`).
        if b.ty == crate::calendar_edit::ENTRY && crate::calendar_edit::is_entry_arg(k) {
            continue;
        }
        let key = blocks::port_key(&b.ty, k);
        // The notification a push notification's block mirrors (#77): by
        // UUID, as `add_block` takes it.
        if k == blocks::MIRRORS
            && let Value::Ref(r) = v
        {
            settings.insert(k.clone(), slug_uuid(r).unwrap_or_else(|| r.clone()));
            continue;
        }
        if blocks::port_name(&b.ty, key).is_some() {
            params.insert(key.to_string(), plain(v));
        } else {
            settings.insert(k.clone(), plain(v));
        }
    }
    (params, settings)
}

/// The settings Properties edits on a block added here (#59): each with how
/// it is written and the value the declaration gives it.
pub fn new_block_settings(b: &NewBlock) -> serde_json::Value {
    blocks::settings(&b.ty)
        .into_iter()
        .map(|(name, kind)| {
            serde_json::json!({
                "name": name,
                "kind": kind,
                "choices": connectors::choice_values(&b.ty, name)
                    .map(|c| c.iter().map(|(w, _)| *w).collect::<Vec<_>>()),
                "value": b.get(name).map(plain),
                // An object of the object tree (#84): the unit lox documents
                // a number in, and whether it cannot go.
                "unit": crate::periphery::of(&b.ty)
                    .and_then(|_| connectors::port_unit(&b.ty, name))
                    .map(unit_suffix),
                "required": crate::periphery::of(&b.ty).is_some_and(|t| t.required.contains(&name)),
            })
        })
        .collect()
}

/// What an op is checked against.
pub struct Ctx<'a> {
    /// The pending config, which the browser showed.
    pub doc: &'a LoxoneDoc,
    /// The project's base: lox compiles the module against it, so a
    /// `page "…"` title must name one of its pages.
    pub base: &'a LoxoneDoc,
    pub lock: &'a Lock,
    /// The deployed revision (the History's: the newest commit of the
    /// base), to tell whether a block added here is deployed. `None`: none.
    pub deployed: Option<&'a LoxoneDoc>,
    /// The names the module's other files declare.
    pub names: &'a BTreeSet<String>,
    /// Of them, the `let` constants (#81): a port may be bound to one.
    pub lets: &'a BTreeSet<String>,
    /// The wires vislox's file drew when the request began: one the
    /// pending config has and an earlier op of the request took out is not
    /// wired any more (#61).
    pub drawn: &'a BTreeSet<Wire>,
    /// What `lox logic adopt` made of the blocks the request adopts (#72),
    /// by UUID, worked out on a scratch copy before the ops are applied.
    pub adoptions: &'a BTreeMap<String, crate::adopt::Adoption>,
    /// For each `adopt_page`, the blocks it adopts, by page UUID.
    pub page_adoptions: &'a BTreeMap<String, Vec<String>>,
    /// The blocks whose switching times (#73), moods (#74), members (#75)
    /// or access rights (#76) the rest of the module claims, per kind: a
    /// block takes one such claim ([`Ctx::claimed`]).
    pub claimed: &'a crate::set_claim::Claimed,
    /// The templates the module's own files declare (#83), by name: an
    /// instance of vislox's may call one.
    pub templates: &'a BTreeMap<String, logic::TemplateDecl>,
    /// The module's own files (#81), read for a request that changes one
    /// of their `let` or `defaults` values, or makes a default.
    pub module: &'a crate::module_edit::ModuleSrc,
}

impl Ctx<'_> {
    /// The blocks the rest of the module claims `kind` of: UUID (lowercase)
    /// → the claim's `file:line`.
    pub fn claimed(&self, kind: &str) -> &BTreeMap<String, String> {
        static NONE: BTreeMap<String, String> = BTreeMap::new();
        self.claimed.get(kind).unwrap_or(&NONE)
    }
}

/// What [`apply_op`] did: the value or op that undoes it (`previous`),
/// and the slug of a block it added.
#[derive(Debug, PartialEq)]
pub struct Applied {
    pub previous: serde_json::Value,
    pub added: Option<String>,
    /// What it could not do and left out, for the reply (`warnings`): a
    /// wire a restored block had whose other end is gone (#80).
    pub warnings: Vec<String>,
}

/// Apply `op` to vislox's files, checked against `ctx`.
pub fn apply_op(files: &mut Files, ctx: &Ctx, op: &Op) -> Result<Applied, String> {
    let done = |p: Option<String>| Applied {
        previous: serde_json::json!(p),
        added: None,
        warnings: Vec::new(),
    };
    match op {
        // A Tree device created on a Tree extension (#85).
        Op::AddBlock { ty, .. } if ty == crate::hw_edit::TREE_DEVICE => {
            crate::hw_edit::add_device(files, ctx, op)
        }
        Op::AddBlock { .. } => add_block(files, ctx, op),
        Op::RemoveBlock { uuid } => {
            // A rule created here (#87).
            if let Some(slug) = crate::rule_edit::created_slug(files, ctx.lock, uuid) {
                return crate::rule_edit::remove(files, ctx, &slug, uuid);
            }
            match crate::hw_edit::created_slug(files, ctx, uuid) {
                Some(slug) => crate::hw_edit::remove_device(files, ctx, &slug, uuid),
                None => remove_block(files, ctx, uuid),
            }
        }
        Op::Rule { uuid, value } => crate::rule_edit::apply(files, ctx, uuid, value.as_ref()),
        Op::NewPage {
            title,
            room,
            category,
            ..
        } => crate::page_edit::new_page(files, ctx, title, room, category.as_deref()),
        Op::RemovePage { page } => crate::page_edit::remove_page(files, ctx, page),
        Op::NewRule { title, value, .. } => {
            crate::rule_edit::new_rule(files, ctx, title, value.as_ref())
        }
        Op::RestoreRule {
            uuid,
            slug,
            title,
            value,
        } => crate::rule_edit::restore(files, ctx, uuid, slug, title, value),
        // A rule created here takes its name in its declaration (#87).
        Op::Title { uuid, value }
            if crate::rule_edit::created_slug(files, ctx.lock, uuid).is_some() =>
        {
            let slug = crate::rule_edit::created_slug(files, ctx.lock, uuid).expect("checked");
            crate::rule_edit::rename(files, ctx, &slug, value.as_deref())
        }
        Op::RestoreBlock { uuid, .. } if crate::hw_edit::restores_device(ctx, uuid) => {
            crate::hw_edit::restore_device(files, ctx, op)
        }
        Op::RestoreBlock { .. } => restore_block(files, ctx, op),
        Op::Statistics { uuid, value } => {
            crate::hw_edit::set_statistics(files, ctx, uuid, value.as_deref())
        }
        Op::Rename { uuid, slug } => rename_block(files, ctx, uuid, slug),
        Op::Tidy => Ok(Applied {
            previous: op_value(&Op::Tidy),
            added: None,
            warnings: Vec::new(),
        }),
        Op::Move { uuid, x, y } => move_block(files, ctx, uuid, *x, *y),
        Op::Relayout { page } => relayout(files, ctx, page),
        Op::Place { moves } => place(files, ctx, moves),
        Op::Wire { from, to } => wire(files, ctx, from, to),
        Op::Unwire { from, to } => unwire(files, ctx, from, to),
        Op::Adopt { blocks } => {
            let uuids: Vec<String> = blocks.iter().map(|b| b.uuid.clone()).collect();
            adopt(files, ctx, &uuids)
        }
        Op::AdoptPage { page } => {
            let uuids = ctx
                .page_adoptions
                .get(page)
                .filter(|u| !u.is_empty())
                .ok_or_else(|| "lox can adopt no block of this page".to_string())?;
            adopt(files, ctx, uuids)
        }
        Op::Unadopt { uuids } => unadopt(files, ctx, uuids),
        Op::Schedule { uuid, value } => crate::sched_edit::apply(files, ctx, uuid, value.as_ref()),
        Op::Moods { uuid, value } => crate::mood_edit::apply(files, ctx, uuid, value.as_ref()),
        Op::Members { uuid, value } => crate::member_edit::apply(files, ctx, uuid, value.as_ref()),
        Op::Access { uuid, value } => crate::access_edit::apply(files, ctx, uuid, value.as_ref()),
        Op::Recipients { uuid, value } => {
            crate::recipients_edit::apply(files, ctx, uuid, value.as_ref())
        }
        Op::Tiles { uuid, value } => crate::tiles_edit::apply(files, ctx, uuid, value.as_ref()),
        // A constant of the module's own files (#81): rewritten there.
        Op::Let {
            name,
            value,
            source,
        } if !files.claims.8.contains_key(name.trim()) && ctx.lets.contains(name.trim()) => {
            crate::module_edit::set_let(
                files,
                ctx,
                name.trim(),
                value.as_deref(),
                source.as_deref(),
            )
            .map(|previous| Applied {
                previous,
                added: None,
                warnings: Vec::new(),
            })
        }
        Op::Let { name, value, .. } => set_constant(files, ctx, name, value.as_deref()).map(done),
        Op::Defaults {
            ty,
            port,
            value,
            source,
            blocks,
        } => crate::module_edit::set_defaults(
            files,
            ctx,
            ty.trim(),
            port.trim(),
            value.as_deref(),
            source.as_deref(),
            blocks.as_deref(),
        )
        .map(|previous| Applied {
            previous,
            added: None,
            warnings: Vec::new(),
        }),
        Op::Expr { uuid, port, value } => {
            crate::expr_edit::set_expr(files, ctx, uuid, port, value.as_deref()).map(done)
        }
        Op::Signal { .. } => crate::expr_edit::set_signal(files, ctx, op),
        Op::Instance { .. } => crate::template_edit::set_instance(files, ctx, op),
        _ => {
            let uuid = op.uuid().unwrap_or_default();
            // A Tree device created here (#85): its name is in its declaration.
            if let Some(slug) = crate::hw_edit::created_slug(files, ctx, uuid) {
                return match op {
                    Op::Title { value, .. } => {
                        crate::hw_edit::set_title(files, &slug, value.as_deref())
                    }
                    _ => Err(format!(
                        "{}: a Tree device created here takes its name here; its other values \
                         once Loxone Config paired it",
                        crate::adopt::name_in(ctx.doc, uuid)
                    )),
                };
            }
            match ctx.lock.slug_of(uuid) {
                Some(slug) if files.block(slug).is_some() => {
                    let slug = slug.to_string();
                    edit_new_block(files, ctx, &slug, op).map(done)
                }
                // A calendar entry's mode and date (#86) are edited only on an
                // entry of vislox's files.
                _ if matches!(op, Op::EntryMode { .. } | Op::EntryDate { .. }) => {
                    Err(crate::calendar_edit::not_ours(files, ctx, uuid))
                }
                // Not in the base, and no block of the module's: an object
                // below a Tree device created here and not deployed yet (#85),
                // whose UUIDs lox mints anew on every compile. An extern of it
                // would name nothing.
                None if op.value().is_some()
                    && find_ci(&ctx.base.xml.root, uuid).is_none()
                    && crate::hw_edit::below_created(files, ctx, uuid) =>
                {
                    Err(format!(
                        "{}: it is not in the project's base config yet; edit it once Apply \
                         deployed it",
                        crate::adopt::name_in(ctx.doc, uuid)
                    ))
                }
                _ => {
                    let managed: HashSet<String> = ctx.lock.slugs.values().cloned().collect();
                    apply(&mut files.claims, ctx.doc, &managed, ctx.lets, op).map(done)
                }
            }
        }
    }
}

/// The ops that undo a request (#62), from each op's `previous`: the last
/// op's first, so a request of `unwire` then `wire` is undone by `unwire`
/// then `wire` of the old ends: one op for each op of the request, so it
/// never holds more than [`MAX_OPS`]. A value op's `previous` is the value
/// it replaced; the others' is the op.
/// `minted` is, per op, the UUID lox minted for the block an `add_block`
/// added: it is undone by `remove_block` of that UUID.
///
/// This is only a convenience over `previous`: the browser sends the list
/// back as an ordinary edit, checked and compiled like any other.
pub fn undo_ops(
    ops: &[Op],
    previous: &[serde_json::Value],
    minted: &[Option<String>],
) -> Result<Vec<Op>, String> {
    let mut undo = Vec::new();
    for (i, op) in ops.iter().enumerate().rev() {
        let prev = previous.get(i).cloned().unwrap_or_default();
        let bad = |e: serde_json::Error| {
            format!(
                "op {} ({}): its previous is no op ({e})",
                i + 1,
                op_name(op)
            )
        };
        match op {
            Op::AddBlock { .. } | Op::NewRule { .. } | Op::NewPage { .. } => {
                match minted.get(i).cloned().flatten() {
                    Some(page) if matches!(op, Op::NewPage { .. }) => {
                        undo.push(Op::RemovePage { page })
                    }
                    Some(uuid) => undo.push(Op::RemoveBlock { uuid }),
                    None => {
                        return Err(format!(
                            "op {} ({}): lox named no UUID for what it added",
                            i + 1,
                            op_name(op)
                        ));
                    }
                }
            }
            Op::RemoveBlock { .. }
            | Op::RestoreBlock { .. }
            | Op::Rename { .. }
            | Op::Tidy
            | Op::Move { .. }
            | Op::Relayout { .. }
            | Op::Place { .. }
            | Op::Wire { .. }
            | Op::Unwire { .. }
            | Op::Adopt { .. }
            | Op::AdoptPage { .. }
            | Op::Signal { .. }
            | Op::Instance { .. }
            | Op::RestoreRule { .. }
            | Op::RemovePage { .. }
            | Op::Defaults { .. }
            | Op::Unadopt { .. } => undo.push(serde_json::from_value(prev).map_err(bad)?),
            // A constant of the module's own files (#81): the op that writes
            // its old text back.
            Op::Let { .. } if prev.is_object() => {
                undo.push(serde_json::from_value(prev).map_err(bad)?)
            }
            Op::Rule { uuid, .. } => undo.push(Op::Rule {
                uuid: uuid.clone(),
                value: serde_json::from_value(prev).map_err(bad)?,
            }),
            Op::Moods { uuid, .. } => undo.push(Op::Moods {
                uuid: uuid.clone(),
                value: serde_json::from_value(prev).map_err(bad)?,
            }),
            Op::Schedule { uuid, .. } => undo.push(Op::Schedule {
                uuid: uuid.clone(),
                value: serde_json::from_value(prev).map_err(bad)?,
            }),
            Op::Members { uuid, .. } => undo.push(Op::Members {
                uuid: uuid.clone(),
                value: serde_json::from_value(prev).map_err(bad)?,
            }),
            Op::Access { uuid, .. } => undo.push(Op::Access {
                uuid: uuid.clone(),
                value: serde_json::from_value(prev).map_err(bad)?,
            }),
            Op::Recipients { uuid, .. } => undo.push(Op::Recipients {
                uuid: uuid.clone(),
                value: serde_json::from_value(prev).map_err(bad)?,
            }),
            Op::Tiles { uuid, .. } => undo.push(Op::Tiles {
                uuid: uuid.clone(),
                value: serde_json::from_value(prev).map_err(bad)?,
            }),
            _ => undo.push(op.with_value(serde_json::from_value(prev).map_err(bad)?)),
        }
    }
    Ok(undo)
}

/// The name an op has in a request (`title`, `add_block`, …).
fn op_name(op: &Op) -> String {
    serde_json::to_value(op)
        .ok()
        .and_then(|v| v["op"].as_str().map(String::from))
        .unwrap_or_default()
}

/// The pages titled `title`, in document order.
pub(crate) fn pages_titled<'a>(el: &'a Element, title: &str, out: &mut Vec<&'a Element>) {
    if el.attr("Type") == Some("Page") && el.attr_decoded("Title").is_some_and(|t| t == title) {
        out.push(el);
    }
    for c in el.child_elements() {
        pages_titled(c, title, out);
    }
}

/// A name for a new block: one line, and as a Loxone Config save keeps it.
pub(crate) fn block_title(ty: &str, t: &str) -> Result<String, String> {
    let t = text(t, "The name", false)?;
    if let Some(w) = connectors::title_save_rewrite(ty, &t) {
        return Err(format!(
            "The name: a Loxone Config save would write it as \"{w}\" (it drops {{ }}, and \
             turns % < | and spaces at either end into _); write that instead"
        ));
    }
    Ok(t)
}

/// A setting's value as lox takes it.
fn setting_value(ty: &str, name: &str, kind: blocks::Kind, v: &str) -> Result<Value, String> {
    use blocks::Kind;
    // An object of the object tree (#84): a long command or pattern is one
    // setting, and a number may carry the unit lox documents (`Poll: 1min`).
    if crate::periphery::of(ty).is_some() {
        match kind {
            Kind::Text => return Ok(Value::Str(crate::periphery::text_value(name, v)?)),
            Kind::Number => {
                return crate::periphery::number_value(ty, name, v);
            }
            _ => {}
        }
    }
    Ok(match kind {
        Kind::Text => Value::Str(text(v, name, connectors::empty_ok(ty, name))?),
        Kind::Choice => {
            let c = connectors::choice_values(ty, name).unwrap_or_default();
            if !c.iter().any(|(w, _)| *w == v) {
                return Err(format!("{name}: one of {}", connectors::choice_list(c)));
            }
            Value::Str(v.to_string())
        }
        Kind::OnOff => match v {
            "on" => Value::Bool(true),
            "off" => Value::Bool(false),
            _ => return Err(format!("{name}: on or off")),
        },
        Kind::Time => Value::Clock(
            logic::clock_seconds(v.trim())
                .and_then(logic::clock_text)
                .ok_or_else(|| format!("{name}: a time of day, like 19:00 or 06:30:15"))?,
        ),
        Kind::Number => Value::Number(number(v).map_err(|e| format!("{name}: {e}"))?),
        Kind::Duration => duration(v.trim())
            .ok_or_else(|| format!("{name}: a duration, like 12h, 30min, 5s or 90 (seconds)"))?,
        // Its file is named once the block's slug is known.
        Kind::Program => Value::File {
            path: String::new(),
            text: Some(program_text(v).map_err(|e| format!("{name}: {e}"))?),
        },
    })
}

/// The most text a program (#79) may hold.
const MAX_PROGRAM: usize = 256 * 1024;

/// The extension of the files vislox keeps a program in (#79).
pub const PROGRAM_EXT: &str = ".c";

/// The file of the program of block `slug`, next to the block's file.
fn program_file(slug: &str) -> String {
    format!("{slug}{PROGRAM_EXT}")
}

/// Whether `name` is a file of vislox's directory: a `.loxl` file, or a
/// program a block of it names (#79).
pub fn ours(name: &str) -> bool {
    !name.starts_with('.') && (name.ends_with(".loxl") || name.ends_with(PROGRAM_EXT))
}

/// A program's text as lox reads it from its file: LF line breaks (lox
/// turns CRLF into LF), no NUL, not empty, not too long.
fn program_text(v: &str) -> Result<String, String> {
    let t = v.replace("\r\n", "\n");
    if t.trim().is_empty() {
        return Err(
            "the program is empty; clear the field to hand it back to Loxone Config".into(),
        );
    }
    if t.contains('\0') || t.contains('\r') {
        return Err("the program holds a character a text file cannot (NUL or a lone CR)".into());
    }
    if t.len() > MAX_PROGRAM {
        return Err(format!(
            "the program is longer than {} KB",
            MAX_PROGRAM / 1024
        ));
    }
    Ok(t)
}

/// A duration as Lox Logic writes one: `12h`, `30min`, `5s`, `500ms`, or
/// seconds as a plain number (lox reads `MinimumTime: 90` as 90 s, its
/// `output_ref_lm_defaults_follow_loxone_config`). Read with lox's own
/// parser, so it is spelled the way lox keeps it. Never negative.
fn duration(v: &str) -> Option<Value> {
    let at = v
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(v.len());
    let (n, u) = v.split_at(at);
    // `at` stops at a sign, so `-5s` leaves `n` empty and is refused here.
    if number(n).is_err() || !["", "ms", "s", "min", "h"].contains(&u) {
        return None;
    }
    if u.is_empty() {
        return Some(Value::Number(n.to_string()));
    }
    crate::units::lox_value(&format!("{n}{u}")).filter(|v| matches!(v, Value::Unit { .. }))
}

fn add_block(files: &mut Files, ctx: &Ctx, op: &Op) -> Result<Applied, String> {
    let Op::AddBlock {
        page,
        ty,
        title,
        params,
        settings,
        wires,
        ..
    } = op
    else {
        unreachable!()
    };
    // An object of the object tree (#84): `page` is its folder or device.
    if let Some(t) = crate::periphery::of(ty) {
        return add_object(files, ctx, op, t);
    }
    // `page "…"` names a page by its title, and lox builds on the first; a
    // page created here (#88) is its file's.
    let (page_title, page_file) = crate::page_edit::target(files, ctx, page, "blocks")?;
    // A push notification drawn on the page (#77): an `OutputRefLM` that
    // mirrors a `Notification` Loxone Config made, with its message.
    let message_ref = ty == blocks::MESSAGE_REF;
    if !blocks::offered(ty) && !message_ref {
        return Err(format!("vislox does not add blocks of type {ty}"));
    }
    let mut settings = settings.clone();
    let mirrors = if message_ref {
        if title.is_some() {
            return Err(
                "a push notification's block takes the notification's name; rename the \
                 notification in Loxone Config"
                    .into(),
            );
        }
        if !params.is_empty() {
            return Err(
                "a push notification's block takes no constants: wire its input from a block"
                    .into(),
            );
        }
        let u = settings
            .remove(blocks::MIRRORS)
            .ok_or("say which push notification it sends (`mirrors`)")?;
        let el = find(&ctx.doc.xml.root, u.trim())
            .filter(|e| e.attr("Type") == Some(blocks::NOTIFICATION))
            .ok_or_else(|| {
                format!(
                    "no push notification {u} in the pending config; make it in Loxone Config \
                     (Messages) first"
                )
            })?;
        let u = el.attr("U").unwrap_or(u.trim());
        let key = extern_key(files, u);
        if let Some(c) = files.claims.0.get(&key)
            && !c.ty.is_empty()
            && c.ty != blocks::NOTIFICATION
        {
            return Err(format!(
                "vislox's file names {u} as a {}, not a push notification",
                c.ty
            ));
        }
        Some(key)
    } else {
        None
    };
    let title = title.as_deref().map(|t| block_title(ty, t)).transpose()?;
    if let Some(key) = params.keys().find(|k| blocks::port_name(ty, k).is_none()) {
        return Err(format!("{ty} has no input {key}"));
    }
    let offered = blocks::settings(ty);
    if let Some(name) = settings
        .keys()
        .find(|n| !offered.iter().any(|(o, _)| o == n))
    {
        return Err(format!("{ty}: {name} cannot be set here"));
    }
    // In the order lox lists them: what it mirrors, the ports, then the
    // settings.
    let mut args = Vec::new();
    if let Some(u) = &mirrors {
        args.push((blocks::MIRRORS.to_string(), Value::Ref(slug(u))));
    }
    for spec in blocks::valued_ports(ty) {
        if let Some(v) = params.get(spec.key) {
            let n = spec.name;
            args.push((
                n.to_string(),
                crate::units::port_value(ty, spec.key, v).map_err(|e| format!("{n}: {e}"))?,
            ));
        }
    }
    for (name, kind) in offered {
        if let Some(v) = settings.get(name) {
            args.push((name.to_string(), setting_value(ty, name, kind, v)?));
        }
    }
    let taken = |s: &str| {
        ctx.names.contains(s)
            || ctx.lock.slugs.contains_key(s)
            || ctx.lock.tombstoned(s)
            || files.names(s)
    };
    let slug = blocks::new_slug(ty, &taken);
    // A program (#79) goes into a file named after the block.
    for (_, v) in &mut args {
        if let Value::File { path, .. } = v {
            *path = program_file(&slug);
        }
    }
    // Everything is checked: from here on `files` changes.
    if let Some(u) = &mirrors {
        // vislox's extern of the notification, kept while the block names it.
        let c = files.claims.0.entry(u.clone()).or_default();
        if c.ty.is_empty() {
            c.ty = blocks::NOTIFICATION.into();
        }
        c.pinned = true;
    }
    // The page's file: by its UUID (it may have been renamed), else a new one.
    let file = match page_file {
        Some(n) => n,
        None => blocks::file_name(&page_title, &|n| n == CLAIMS || files.pages.contains_key(n)),
    };
    let f = files.pages.entry(file).or_insert_with(|| PageFile {
        uuid: Some(page.clone()),
        ..PageFile::new(&page_title)
    });
    f.page = page_title;
    let b = NewBlock {
        slug: slug.clone(),
        ty: ty.clone(),
        title,
        args,
        ..Default::default()
    };
    f.blocks.push(b.clone());
    // Its wires (an undone removal's): an empty UUID is this block.
    let mut geo = Geo::new();
    for w in wires {
        let mut at = |e: &[String; 2]| {
            if e[0].is_empty() {
                new_end(&b, &e[1])
            } else {
                end(files, ctx, e, &mut geo)
            }
        };
        let (src, dst) = (at(&w.from)?, at(&w.to)?);
        connect(files, ctx, &src, &dst)?;
    }
    Ok(Applied {
        previous: serde_json::Value::Null,
        added: Some(slug),
        warnings: Vec::new(),
    })
}

/// Add an object of the object tree (#84): a virtual input, a virtual or
/// Modbus device or one of their I/Os, declared in vislox's periphery file
/// ([`crate::periphery::FILE`]). `page` is the folder it goes into or the
/// device it hangs below; lox mints its UUIDs and `IName=` and puts it there.
fn add_object(
    files: &mut Files,
    ctx: &Ctx,
    op: &Op,
    t: &crate::periphery::Type,
) -> Result<Applied, String> {
    let Op::AddBlock {
        page: parent,
        ty,
        title,
        params,
        settings,
        wires,
        ..
    } = op
    else {
        unreachable!()
    };
    let at = crate::periphery::parent(files, ctx, t, parent)?;
    if !params.is_empty() {
        return Err(format!(
            "a {ty} takes no constants here: wire its input once it is in the config"
        ));
    }
    if !wires.is_empty() {
        return Err(format!(
            "a {ty} is added without wires: draw them once it is in the config"
        ));
    }
    let title = title.as_deref().map(|t| block_title(ty, t)).transpose()?;
    // An operating mode (#86) has a name no other mode has.
    if ty == crate::calendar_edit::MODE {
        let t = title
            .as_deref()
            .ok_or("a new operating mode needs its name")?;
        if let Some(e) = crate::calendar_edit::mode_title_taken(files, ctx, t, None) {
            return Err(format!("{t}: {e}"));
        }
    }
    // A calendar entry (#86): its mode and its date, never stored attributes.
    let entry = ty == crate::calendar_edit::ENTRY;
    let offered = blocks::settings(ty);
    if let Some(name) = settings
        .keys()
        .find(|n| !entry && !offered.iter().any(|(o, _)| o == n))
    {
        return Err(format!("{ty}: {name} cannot be set here"));
    }
    let mut args = if entry {
        crate::calendar_edit::new_args(ctx, settings)?
    } else {
        Vec::new()
    };
    if let Some(d) = at.name() {
        args.push((crate::periphery::DEVICE.to_string(), Value::Ref(d)));
    }
    for (name, kind) in offered {
        if let Some(v) = settings.get(name) {
            args.push((name.to_string(), setting_value(ty, name, kind, v)?));
        }
    }
    let missing = crate::periphery::missing(ty, &args);
    if !missing.is_empty() {
        return Err(format!(
            "a new {ty} needs {}: lox makes none without",
            missing.join(" and ")
        ));
    }
    if let Some(e) = crate::periphery::rewritten(ty, &args) {
        return Err(e);
    }
    let taken = |s: &str| {
        ctx.names.contains(s)
            || ctx.lock.slugs.contains_key(s)
            || ctx.lock.tombstoned(s)
            || files.names(s)
    };
    let slug = blocks::new_slug(ty, &taken);
    // Everything is checked: from here on `files` changes.
    if let crate::periphery::Parent::Theirs(key, pty) = &at {
        // vislox's extern of the device, kept while an I/O names it.
        let c = files.claims.0.entry(key.clone()).or_default();
        if c.ty.is_empty() {
            c.ty = pty.clone();
        }
        c.pinned = true;
    }
    let f = files
        .pages
        .entry(crate::periphery::FILE.to_string())
        .or_insert_with(PageFile::periphery);
    f.blocks.push(NewBlock {
        slug: slug.clone(),
        ty: ty.clone(),
        title,
        args,
        ..Default::default()
    });
    Ok(Applied {
        previous: serde_json::Value::Null,
        added: Some(slug),
        warnings: Vec::new(),
    })
}

/// The `page` of the block `b` of vislox's file `file`, as `add_block` and
/// `restore_block` take it: its page, or for an object of the object tree
/// (#84) its folder or device.
fn block_page(files: &Files, ctx: &Ctx, file: &str, b: &NewBlock) -> String {
    if files.pages[file].periphery {
        crate::periphery::parent_uuid(files, ctx, b)
    } else {
        file_page(files, ctx, file)
    }
}

fn remove_block(files: &mut Files, ctx: &Ctx, uuid: &str) -> Result<Applied, String> {
    let r = removal(files, ctx, uuid, &crate::periphery::below(files))?;
    if r.delete {
        return delete_block(files, ctx, &r.slug, r.wires);
    }
    let slug = r.slug;
    let (file, b) = files.block(&slug).expect("checked");
    let (file, b) = (file.to_string(), b.clone());
    let (params, mut settings) = split_args(&b);
    // A calendar entry (#86) is added again with its mode by UUID.
    if b.ty == crate::calendar_edit::ENTRY {
        settings = crate::calendar_edit::again_settings(ctx, &b);
    }
    let again = Op::AddBlock {
        page: block_page(files, ctx, &file, &b),
        ty: b.ty.clone(),
        title: b.title.clone(),
        params,
        settings,
        // Its wires, which go with it: drawn again when it is added again.
        wires: r.wires,
        restores: Some(uuid.to_string()),
    };
    let p = files.pages.get_mut(&file).expect("found");
    p.blocks.retain(|x| x.slug != slug);
    // Its wires go with it, as in Loxone Config; so does the extern of the
    // notification it mirrored (#77), when nothing else names it.
    files.claims.1.retain(|w| !w.touches(&slug));
    files.repin();
    let p = files.pages.get_mut(&file).expect("found");
    p.removed.insert(slug);
    Ok(Applied {
        previous: op_value(&again),
        added: None,
        warnings: Vec::new(),
    })
}

// ── deleting and renaming managed blocks (#80) ─────────────────────────

/// How `remove_block` of a block goes, worked out without changing anything.
struct Removal {
    slug: String,
    /// Deployed or adopted: deleted from the config (`removed`), undone by
    /// `restore_block`. Else taken out, undone by `add_block`.
    delete: bool,
    /// The wires drawn here at it, which go with it.
    wires: Vec<Ends>,
}

/// Whether `remove_block` of `uuid` is refused now, and why (`None`: it is
/// not). Changes nothing: `GET /api/edits` asks it for every block
/// ([`delete_refusals`]).
#[cfg(test)]
pub fn delete_refusal(files: &Files, ctx: &Ctx, uuid: &str) -> Option<String> {
    removal(files, ctx, uuid, &crate::periphery::below(files)).err()
}

/// [`delete_refusal`] of each of `uuids`, with what hangs below each device
/// (#84) worked out once.
pub fn delete_refusals(files: &Files, ctx: &Ctx, uuids: &[String]) -> Vec<Option<String>> {
    let below = crate::periphery::below(files);
    uuids
        .iter()
        .map(|u| match crate::hw_edit::created_slug(files, ctx, u) {
            // A Tree device created here (#85).
            Some(_) => crate::hw_edit::removal(files, ctx, u).err(),
            None => removal(files, ctx, u, &below).err(),
        })
        .collect()
}

fn removal(
    files: &Files,
    ctx: &Ctx,
    uuid: &str,
    below: &crate::periphery::Below,
) -> Result<Removal, String> {
    let el = find_ci(&ctx.doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let name = el.attr_decoded("Title").unwrap_or_default().into_owned();
    let slug = match ctx.lock.slug_of(uuid) {
        Some(s) if files.block(s).is_some() => s.to_string(),
        Some(_) => {
            return Err(format!(
                "{name}: the module manages this block; delete it in its .loxl file"
            ));
        }
        None => {
            return Err(format!(
                "{name}: only a block added or adopted here can be deleted here"
            ));
        }
    };
    // An operating mode an entry of vislox's switches on (#86): lox would
    // find no mode for the entry.
    // So would Loxone Config's entries that name its number (`CalType=`).
    let is_mode = files
        .block(&slug)
        .is_some_and(|(_, b)| b.ty == crate::calendar_edit::MODE);
    let by = if is_mode {
        crate::calendar_edit::switched_by(files, ctx, &slug, uuid)
    } else {
        Vec::new()
    };
    if !by.is_empty() {
        return Err(format!(
            "{name}: the calendar {} {} switch{} it on; change or delete {} first",
            if by.len() == 1 { "entry" } else { "entries" },
            by.join(", "),
            if by.len() == 1 { "es" } else { "" },
            if by.len() == 1 { "it" } else { "them" },
        ));
    }
    // An expression or a wire of vislox's that reads it (#82) would name nothing.
    if let Some(what) = crate::expr_edit::read_by_expression(files, &slug) {
        return Err(format!("{name}: {what} reads it; take that out first"));
    }
    if let Some((what, them)) = files.claims.claim_on(&slug) {
        return Err(format!(
            "{name}: vislox's file claims {what}; take {them} out first (Revert them, or undo the \
             edit)"
        ));
    }
    // A device the objects of vislox's files hang below (#84): lox would
    // find no device for them.
    if let Some(o) = below.get(&slug).and_then(|v| v.first()) {
        let other = ctx
            .lock
            .uuid_of(o)
            .and_then(|u| find_ci(&ctx.doc.xml.root, u))
            .and_then(|e| e.attr_decoded("Title").map(|t| t.into_owned()))
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| o.to_string());
        return Err(format!("{name}: {other} hangs below it; delete that first"));
    }
    // Objects Loxone Config made below a device of vislox's would go with
    // it, and whatever reads them: refused, naming them.
    let others = crate::periphery::others_below(files, ctx, uuid);
    if !others.is_empty() {
        return Err(format!(
            "{name}: {} below it {} made in Loxone Config, and would be deleted with it; delete \
             or move {} there first",
            others.join(", "),
            if others.len() == 1 { "was" } else { "were" },
            if others.len() == 1 { "it" } else { "them" },
        ));
    }
    let (_, b) = files.block(&slug).expect("checked");
    let wires = wires_at(files, ctx, &slug, &b.ty).map_err(|w| {
        format!(
            "{name}: the wire `{w}` drawn here names an end vislox cannot find; take it out first"
        )
    })?;
    let delete = b.adopted
        || ctx
            .deployed
            .is_some_and(|d| find_ci(&d.xml.root, uuid).is_some());
    if delete {
        // A block whose declaration takes a wire from it would lose that wire,
        // and its declaration is its own: delete that one first.
        for p in files.pages.values() {
            for o in p.blocks.iter().filter(|o| o.slug != slug) {
                if let Some((port, _)) = o.wires.iter().find(|(_, src)| src.slug == slug) {
                    let other = ctx
                        .lock
                        .uuid_of(&o.slug)
                        .and_then(|u| find_ci(&ctx.doc.xml.root, u))
                        .and_then(|e| e.attr_decoded("Title").map(|t| t.into_owned()))
                        .filter(|t| !t.trim().is_empty())
                        .unwrap_or_else(|| o.slug.clone());
                    return Err(format!(
                        "{name}: {other} takes a wire from it in its declaration ({port}); delete \
                         {other} first, or hand it back to Loxone Config"
                    ));
                }
            }
            if let Some(w) = p
                .wires
                .iter()
                .find(|w| w.to.slug == slug && w.from.slug != slug)
            {
                return Err(format!(
                    "{name}: the wire `{}` of vislox's files ends at it; take it out first",
                    w.render()
                ));
            }
        }
        if ctx.lock.entry(&slug).is_none() {
            return Err(format!(
                "{name}: the lockfile has no entry for `{slug}`; compile the module first"
            ));
        }
    }
    Ok(Removal {
        slug,
        delete,
        wires,
    })
}

/// The page UUID of vislox's page file `file`: its `# page uuid:` line, else
/// the first page of the base with its title.
fn file_page(files: &Files, ctx: &Ctx, file: &str) -> String {
    let pf = &files.pages[file];
    // A page created here (#88): the UUID the lock keeps, not a stale line.
    match crate::page_edit::created_uuid(pf, ctx.lock).or_else(|| pf.uuid.clone()) {
        Some(u) => u,
        None => {
            let mut same = Vec::new();
            pages_titled(&ctx.base.xml.root, &pf.page, &mut same);
            same.first()
                .and_then(|e| e.attr("U"))
                .unwrap_or_default()
                .to_string()
        }
    }
}

/// Every wire drawn here (#61) at the block `slug` of type `ty`, by its ends
/// as `add_block` and `restore_block` take them (an empty UUID for the block
/// itself). `Err`: a wire whose other end vislox cannot name by UUID (as
/// written), which an undo could not draw again.
fn wires_at(files: &Files, ctx: &Ctx, slug: &str, ty: &str) -> Result<Vec<Ends>, String> {
    let here = |e: &(String, String)| -> Option<[String; 2]> {
        if e.0 == slug {
            Some([String::new(), port_names::key_or_raw(ty, &e.1).to_string()])
        } else {
            wire_end_at(files, ctx.lock, e)
        }
    };
    files
        .claims
        .1
        .iter()
        .filter(|w| w.touches(slug))
        .map(|w| match (here(&w.from), here(&w.to)) {
            (Some(from), Some(to)) => Ok(Ends { from, to }),
            _ => Err(w.render()),
        })
        .collect()
}

/// A lockfile entry's port UUIDs (and its own), lowercase.
fn port_set(entry: &serde_json::Value) -> BTreeSet<String> {
    crate::adopt::ports_of(entry)
        .into_iter()
        .map(|u| u.to_ascii_lowercase())
        .collect()
}

/// Delete a deployed or adopted block of vislox's files ([`removal`] checked
/// it): its declaration goes, `removed <slug>` tells lox to take it out of
/// the config (lox keeps a tombstone, and the push deletes it on the
/// Miniserver), and the wires drawn here at it go with it. Its `previous` is
/// the `restore_block` that brings it back with its UUIDs while the tombstone
/// is there.
fn delete_block(
    files: &mut Files,
    ctx: &Ctx,
    slug: &str,
    wires: Vec<Ends>,
) -> Result<Applied, String> {
    let (file, b) = files.block(slug).expect("checked");
    let (file, b) = (file.to_string(), b.clone());
    let entry = ctx.lock.entry(slug).cloned().expect("checked");
    let ports = port_set(&entry);
    let lock_wires: Vec<serde_json::Value> = ctx
        .lock
        .wires
        .iter()
        .filter(|(from, _)| ports.contains(&from.to_ascii_lowercase()))
        .map(|(from, to)| serde_json::json!({ "from": from, "to": to }))
        .collect();
    let out_wires: Vec<&blocks::FileWire> = files.pages[&file]
        .wires
        .iter()
        .filter(|w| w.from.slug == slug)
        .collect();
    let mut source = b.render();
    source.push('\n');
    for w in &out_wires {
        source.push_str(&w.render());
        source.push('\n');
    }
    let program = b.args.iter().find_map(|(_, v)| match v {
        Value::File { text: Some(t), .. } => Some(t.clone()),
        _ => None,
    });
    // vislox's externs its source names: brought back with it.
    let mut named: Vec<String> = b.wires.iter().map(|(_, src)| src.slug.clone()).collect();
    named.extend(out_wires.iter().map(|w| w.to.slug.clone()));
    for k in [blocks::MIRRORS, crate::periphery::DEVICE] {
        if let Some(Value::Ref(n)) = b.get(k) {
            named.push(n.clone());
        }
    }
    let externs: BTreeMap<String, String> = files
        .claims
        .0
        .iter()
        .filter(|(u, _)| named.contains(&self::slug(u)))
        .map(|(u, c)| (u.clone(), c.ty.clone()))
        .collect();
    let again = Op::RestoreBlock {
        uuid: ctx.lock.uuid_of(slug).unwrap_or_default().to_string(),
        page: block_page(files, ctx, &file, &b),
        adopted: b.adopted,
        source,
        program,
        wires,
        externs,
        lock: entry,
        lock_wires,
    };
    // Everything is checked: from here on `files` changes.
    let p = files.pages.get_mut(&file).expect("found");
    p.blocks.retain(|x| x.slug != slug);
    p.wires.retain(|w| w.from.slug != slug);
    p.removed.insert(slug.to_string());
    files.claims.1.retain(|w| !w.touches(slug));
    files.layout.moves.remove(slug);
    files.repin();
    Ok(Applied {
        previous: op_value(&again),
        added: None,
        warnings: Vec::new(),
    })
}

/// The fields lox writes into a managed block's lockfile entry.
const ENTRY_FIELDS: &[&str] = &[
    "uuid",
    "type",
    "ports",
    "layout",
    "superseded_layout",
    "page_uuid",
    "gui_defaults",
    "expr_owned",
    "states",
];

/// Check the lockfile entry a `restore_block` brings against what it is
/// for: the tombstone (its UUID and type), the declaration's type, and the
/// object the base has under that UUID: each port is one of that object's
/// connectors (by key and UUID), and its page is the page it is drawn on.
/// So an op that was tampered with, or is stale, cannot pin another
/// object's UUIDs.
fn check_entry(
    ctx: &Ctx,
    tomb_uuid: &str,
    ty: &str,
    page: &str,
    entry: &serde_json::Value,
) -> Result<(), String> {
    let bad = |why: String| Err(format!("its lockfile entry {why}"));
    let Some(o) = entry.as_object() else {
        return bad("is not an object".into());
    };
    if let Some(k) = o.keys().find(|k| !ENTRY_FIELDS.contains(&k.as_str())) {
        return bad(format!("has `{k}`, which lox does not write"));
    }
    if !o
        .get("uuid")
        .and_then(|u| u.as_str())
        .is_some_and(|u| u.eq_ignore_ascii_case(tomb_uuid))
    {
        return bad(format!("is not {tomb_uuid}'s"));
    }
    if o.get("type").and_then(|t| t.as_str()) != Some(ty) {
        return bad(format!("is not a {ty}'s"));
    }
    if o.get("expr_owned")
        .is_some_and(|x| x != &serde_json::Value::Bool(false))
    {
        return bad("is an expression's".into());
    }
    let el = find_ci(&ctx.base.xml.root, tomb_uuid)
        .ok_or_else(|| format!("the project's base has no object {tomb_uuid} any more"))?;
    let none = serde_json::Map::new();
    let ports = match o.get("ports") {
        None => &none,
        Some(p) => match p.as_object() {
            Some(p) => p,
            None => return bad("has ports that are not a map".into()),
        },
    };
    for (k, u) in ports {
        let ok = u.as_str().is_some_and(|u| {
            el.child_elements().any(|c| {
                c.name == "Co"
                    && c.attr("K") == Some(k.as_str())
                    && c.attr("U").is_some_and(|x| x.eq_ignore_ascii_case(u))
            })
        });
        if !ok {
            return bad(format!(
                "names port {k} {u}, which is not one of its connectors"
            ));
        }
    }
    for f in ["layout", "superseded_layout"] {
        let Some(l) = o.get(f) else { continue };
        // An object of the object tree (#84) has no position.
        if l.is_null() && connectors::object_tree(ty) {
            continue;
        }
        let n = |k: &str| l.get(k).and_then(|v| v.as_i64());
        let ok = match (n("px"), n("py"), n("px2"), n("py2")) {
            (Some(x), Some(y), Some(x2), Some(y2)) => {
                [x, y, x2, y2].iter().all(|v| v.abs() <= MAX_COORD) && x < x2 && y < y2
            }
            _ => false,
        };
        if !ok {
            return bad(format!("has no rectangle as its {f}"));
        }
    }
    if let Some(pu) = o.get("page_uuid") {
        let drawn = el
            .attr("U")
            .and_then(|u| drawn_at(ctx.base, u))
            .map(|(p, _)| p);
        let ok = pu.as_str().is_some_and(|pu| {
            pu.eq_ignore_ascii_case(page) && drawn.is_some_and(|d| d.eq_ignore_ascii_case(pu))
        });
        if !ok {
            return bad("names another page than the one it is drawn on".into());
        }
    }
    if o.get("gui_defaults").is_some_and(|g| !g.is_boolean()) {
        return bad("has gui_defaults that are not true or false".into());
    }
    if o.get("states").is_some_and(|s| {
        !s.as_array()
            .is_some_and(|a| a.iter().all(serde_json::Value::is_string))
    }) {
        return bad("has states that are not names".into());
    }
    Ok(())
}

/// Draw one wire of a restored block `b` again, as `add_block` does.
fn redraw(files: &mut Files, ctx: &Ctx, b: &NewBlock, w: &Ends) -> Result<(), String> {
    let mut geo = Geo::new();
    let mut at = |files: &Files, e: &[String; 2]| {
        if e[0].is_empty() {
            new_end(b, &e[1])
        } else {
            end(files, ctx, e, &mut geo)
        }
    };
    let src = at(files, &w.from)?;
    let dst = at(files, &w.to)?;
    connect(files, ctx, &src, &dst)
}

/// Bring back a block [`delete_block`] deleted, while lox keeps its
/// tombstone: its declaration, its lockfile entry and the tombstone's
/// removal (so lox writes its old UUIDs, layout and wires again), and the
/// wires drawn here at it. A wire whose other end is gone is left out and
/// said (`warnings`): the block comes back all the same.
fn restore_block(files: &mut Files, ctx: &Ctx, op: &Op) -> Result<Applied, String> {
    let Op::RestoreBlock {
        uuid,
        page,
        adopted,
        source,
        program,
        wires,
        externs,
        lock,
        lock_wires,
    } = op
    else {
        unreachable!()
    };
    let (tomb_uuid, slug, ty) = match ctx.lock.tomb(uuid) {
        Some((u, s, t)) => (u.to_string(), s.to_string(), t.to_string()),
        None if ctx.lock.slug_of(uuid).is_some() => {
            return Err(format!(
                "{uuid} is not deleted: there is nothing to bring back"
            ));
        }
        None => {
            return Err(format!(
                "lox keeps no tombstone of {uuid} any more (its deletion was applied or \
                 discarded): it cannot come back with its UUIDs"
            ));
        }
    };
    if files.block(&slug).is_some() || ctx.lock.slugs.contains_key(&slug) {
        return Err(format!(
            "`{slug}` names another block now; rename that one first"
        ));
    }
    if find_ci(&ctx.base.xml.root, uuid).is_none() {
        return Err(format!(
            "the project's base has no object {uuid} any more: it cannot come back with its UUIDs"
        ));
    }
    let consts: BTreeSet<String> = files.claims.8.keys().cloned().collect();
    // An object of the object tree (#84) is in vislox's periphery file, which
    // has no page: `page` is its folder or device.
    let adopted_line = if *adopted {
        format!("# {}\n", blocks::ADOPTED_LINE)
    } else {
        String::new()
    };
    // Read once as the periphery file reads it, when it may be one.
    let periph = (crate::periphery::of(&ty).is_some() || ty.is_empty())
        .then(|| PageFile::parse_periphery(&format!("{adopted_line}{source}\n"), &consts));
    let object = crate::periphery::of(&ty).is_some()
        || (ty.is_empty() && periph.as_ref().is_some_and(Result::is_ok));
    let page_title = if object {
        String::new()
    } else {
        let pg = find(&ctx.base.xml.root, page)
            .filter(|e| e.attr("Type") == Some("Page"))
            .ok_or_else(|| format!("no page {page} in the project's base config"))?;
        let page_title = pg.attr_decoded("Title").unwrap_or_default().into_owned();
        let mut same = Vec::new();
        pages_titled(&ctx.base.xml.root, &page_title, &mut same);
        if page_title.trim().is_empty()
            || same.first().and_then(|e| e.attr("U")) != Some(page.as_str())
        {
            return Err(format!(
                "lox puts a block on the first page titled \"{page_title}\", and that is not its \
                 page any more"
            ));
        }
        page_title
    };
    // Read as vislox reads its files: only what it writes.
    let pf = if let (true, Some(p)) = (object, periph) {
        p
    } else {
        PageFile::parse_with(
            &format!(
                "{}\n{adopted_line}{source}\n",
                blocks::render_page_line(&page_title)
            ),
            &consts,
        )
    }
    .map_err(|e| format!("its declaration: {e}"))?;
    let [b] = pf.blocks.as_slice() else {
        return Err("its declaration is one block".into());
    };
    if b.slug != slug || !pf.removed.is_empty() || !pf.moved.is_empty() {
        return Err(format!("its declaration is not `{slug}`'s"));
    }
    if !ty.is_empty() && b.ty != ty {
        return Err(format!("`{slug}` was a {ty}, not a {}", b.ty));
    }
    if pf.wires.iter().any(|w| w.from.slug != slug) {
        return Err("its declaration's wires are out of it only".into());
    }
    check_entry(ctx, &tomb_uuid, &b.ty, page, lock)?;
    let ports = port_set(lock);
    for w in lock_wires {
        let from = w.get("from").and_then(|f| f.as_str()).unwrap_or("");
        if w.get("to").and_then(|t| t.as_str()).is_none()
            || !ports.contains(&from.to_ascii_lowercase())
        {
            return Err("its lockfile wires are out of its ports only".into());
        }
    }
    for (u, t) in externs {
        if t.trim().is_empty() || find_ci(&ctx.doc.xml.root, u).is_none() {
            return Err(format!("no object {u} in the pending config"));
        }
    }
    // Every name its declaration uses: an extern it brings, one of the
    // claims file, or a block of vislox's files.
    let mut named: Vec<&str> = b.wires.iter().map(|(_, src)| src.slug.as_str()).collect();
    named.extend(pf.wires.iter().map(|w| w.to.slug.as_str()));
    for k in [blocks::MIRRORS, crate::periphery::DEVICE] {
        if let Some(Value::Ref(n)) = b.get(k) {
            named.push(n);
        }
    }
    for n in named {
        let ext = externs
            .keys()
            .chain(files.claims.0.keys())
            .any(|u| self::slug(u) == n);
        if n != slug && !ext && files.block(n).is_none() {
            return Err(format!(
                "its declaration names `{n}`, which is neither an extern of vislox's files nor \
                 a block of them"
            ));
        }
    }
    // Where an object of the object tree goes is what its declaration says.
    if object {
        let now = crate::periphery::parent_uuid(files, ctx, b);
        let ext = |n: &str| externs.keys().find(|u| self::slug(u) == n).cloned();
        let now = match (now.is_empty(), b.get(crate::periphery::DEVICE)) {
            (true, Some(Value::Ref(n))) => ext(n).unwrap_or_default(),
            _ => now,
        };
        if now.is_empty() || !now.eq_ignore_ascii_case(page) {
            return Err(format!(
                "its declaration puts it elsewhere than {page}, or there is no such folder or device"
            ));
        }
    }
    let mut b = b.clone();
    let taken = files.programs();
    for (_, v) in b.args.iter_mut() {
        if let Value::File { path, text } = v {
            if !ours(path) || !path.ends_with(PROGRAM_EXT) || taken.contains_key(path.as_str()) {
                return Err(format!("its program file {path} is another block's"));
            }
            let t = program
                .as_deref()
                .ok_or("its declaration names a program, and the op has none")?;
            *text = Some(program_text(t)?);
        }
    }
    // Everything is checked: from here on `files` changes.
    for (u, t) in externs {
        let c = files.claims.0.entry(extern_key(files, u)).or_default();
        if c.ty.is_empty() {
            c.ty = t.clone();
        }
    }
    let file = if object {
        crate::periphery::FILE.to_string()
    } else {
        match files
            .pages
            .iter()
            .find(|(_, p)| p.uuid.as_deref() == Some(page.as_str()))
        {
            Some((n, _)) => n.clone(),
            None => blocks::file_name(&page_title, &|n| n == CLAIMS || files.pages.contains_key(n)),
        }
    };
    for p in files.pages.values_mut() {
        p.removed.remove(&slug);
    }
    let f = files.pages.entry(file).or_insert_with(|| {
        if object {
            PageFile::periphery()
        } else {
            PageFile {
                uuid: Some(page.clone()),
                ..PageFile::new(&page_title)
            }
        }
    });
    if !object {
        f.page = page_title;
    }
    f.blocks.push(b.clone());
    f.wires.extend(pf.wires.iter().cloned());
    // Each wire on a copy: one whose other end is gone is left out, said,
    // and the block comes back all the same.
    let mut warnings = Vec::new();
    for w in wires {
        let mut t = files.clone();
        match redraw(&mut t, ctx, &b, w) {
            Ok(()) => *files = t,
            Err(e) => warnings.push(format!(
                "the wire {} → {} was not drawn again: {e}",
                w.from.join(" "),
                w.to.join(" ")
            )),
        }
    }
    files.repin();
    let le = &mut files.lock_edits;
    le.add.insert(slug.clone(), lock.clone());
    le.untomb.insert(tomb_uuid);
    le.wires.extend(lock_wires.iter().cloned());
    Ok(Applied {
        previous: op_value(&Op::RemoveBlock { uuid: uuid.clone() }),
        added: None,
        warnings,
    })
}

/// Rename the block `uuid` of vislox's files to `new` (#80): lox's own
/// rename (`rename_slug`) over vislox's files, and `moved <old> -> <new>`, so
/// lox moves its lockfile entry and it keeps its UUIDs. A block without a
/// name shows its slug: it keeps the old one as its name, so nothing in the
/// config changes; renamed back, it shows its slug again.
fn rename_block(files: &mut Files, ctx: &Ctx, uuid: &str, new: &str) -> Result<Applied, String> {
    let name = find_ci(&ctx.doc.xml.root, uuid)
        .and_then(|e| e.attr_decoded("Title").map(|t| t.into_owned()))
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| uuid.to_string());
    let old = match ctx.lock.slug_of(uuid) {
        Some(s) if files.block(s).is_some() => s.to_string(),
        Some(s) => {
            return Err(format!(
                "{name}: the module manages it as `{s}` in its own .loxl file; rename it there \
                 (lox logic rename)"
            ));
        }
        None => {
            return Err(format!(
                "{name}: only a block added or adopted here is renamed here"
            ));
        }
    };
    let new = new.trim();
    if new == old {
        return Err(format!("{name} is `{new}` already"));
    }
    logic::valid_slug(new).map_err(|e| e.to_string())?;
    if !new.starts_with(SLUG_PREFIX) || new.len() == SLUG_PREFIX.len() {
        return Err(format!(
            "vislox's blocks are named {SLUG_PREFIX} and then lowercase letters, digits and _"
        ));
    }
    if new.contains("__") {
        return Err("`__` is lox's for the blocks an expression makes".into());
    }
    if slug_uuid(new).is_some() {
        return Err(format!(
            "`{new}` is how vislox names an object of Loxone Config's"
        ));
    }
    if ctx.names.contains(new)
        || ctx.lock.slugs.contains_key(new)
        || ctx.lock.tombstoned(new)
        || files.names(new)
    {
        return Err(format!(
            "`{new}` is taken: the module or its lockfile has that name"
        ));
    }
    // lox's rename, file by file: declarations, wires, set claims, targets.
    let mut texts = files.render();
    for t in texts.values_mut() {
        let mut m = Module::parse_fragment(t).map_err(|e| e.to_string())?;
        logic::rename_slug(&mut m, &old, new);
        *t = m.to_text();
    }
    for (n, t) in files.programs() {
        texts.insert(n.to_string(), t.to_string());
    }
    let mut renamed = Files::from_texts(&texts)
        .map_err(|e| format!("vislox's files do not read back after the rename: {e}"))?;
    renamed.present = files.present.clone();
    renamed.layout = files.layout.clone();
    renamed.lock_edits = files.lock_edits.clone();
    // A rename of a rename lox has not applied yet is one rename; back to the
    // old slug, none.
    let file = renamed
        .block(new)
        .map(|(f, _)| f.to_string())
        .ok_or_else(|| format!("lox's rename left no block `{new}` in vislox's files"))?;
    let earlier = renamed.pages.values().find_map(|p| {
        p.moved
            .iter()
            .find(|(_, t)| t.as_str() == new)
            .map(|(f, _)| f.clone())
    });
    for p in renamed.pages.values_mut() {
        p.moved.retain(|_, t| t != new);
    }
    match earlier {
        Some(from) if from == new => {}
        Some(from) => {
            if let Some(p) = renamed.pages.get_mut(&file) {
                p.moved.insert(from, new.to_string());
            }
        }
        None => {
            if let Some(p) = renamed.pages.get_mut(&file) {
                p.moved.insert(old.clone(), new.to_string());
            }
        }
    }
    let b = renamed
        .block_mut(new)
        .ok_or_else(|| format!("lox's rename left no block `{new}` in vislox's files"))?;
    if b.ty != blocks::MESSAGE_REF {
        match &b.title {
            None => b.title = Some(old.clone()),
            Some(t) if t == new => b.title = None,
            Some(_) => {}
        }
    }
    for (_, v) in b.args.iter_mut() {
        if let Value::File { path, .. } = v
            && *path == program_file(&old)
        {
            *path = program_file(new);
        }
    }
    *files = renamed;
    Ok(Applied {
        previous: op_value(&Op::Rename {
            uuid: uuid.to_string(),
            slug: old,
        }),
        added: None,
        warnings: Vec::new(),
    })
}

// ── adopting blocks (#72) ───────────────────────────────────────────────

/// Declare the blocks `uuids` in vislox's files, as [`crate::adopt`] worked
/// them out on the scratch copy: each in the file of its page, its externs in
/// [`FILE`], its lockfile entry and tracked wires for the lock.
fn adopt(files: &mut Files, ctx: &Ctx, uuids: &[String]) -> Result<Applied, String> {
    for u in uuids {
        let a = ctx
            .adoptions
            .get(u)
            .ok_or_else(|| format!("{u} was not adopted on the scratch copy"))?;
        if files.block(&a.slug).is_some() {
            return Err(format!("`{}` is a block of vislox's files already", a.slug));
        }
        // `page "…"` names a page by its title, and lox builds on the first.
        let mut same = Vec::new();
        pages_titled(&ctx.base.xml.root, &a.page_title, &mut same);
        if same.first().and_then(|e| e.attr("U")) != Some(a.page.as_str()) {
            return Err(format!(
                "{} pages are titled \"{}\", and lox puts a page's blocks on the first; rename one \
                 in Loxone Config to adopt blocks there",
                same.len(),
                a.page_title
            ));
        }
        let file = match files
            .pages
            .iter()
            .find(|(_, p)| p.uuid.as_deref() == Some(a.page.as_str()))
        {
            Some((n, _)) => n.clone(),
            None => blocks::file_name(&a.page_title, &|n| {
                n == CLAIMS || files.pages.contains_key(n)
            }),
        };
        let f = files.pages.entry(file).or_insert_with(|| PageFile {
            uuid: Some(a.page.clone()),
            ..PageFile::new(&a.page_title)
        });
        f.page = a.page_title.clone();
        f.blocks.push(a.block.clone());
        f.wires.extend(a.wires.iter().cloned());
        for (eu, ty) in &a.externs {
            let c = files.claims.0.entry(eu.clone()).or_default();
            if c.ty.is_empty() {
                c.ty = ty.clone();
            }
            c.pinned = true;
        }
        // Adopted under vislox's extern of it: the wires blocks adopted earlier
        // drew onto it are its declaration's now, and they name it by its slug.
        if a.promoted {
            let ext = slug(u);
            for p in files.pages.values_mut() {
                p.wires.retain(|w| w.to.slug != ext);
                for b in p.blocks.iter_mut() {
                    for (_, src) in b.wires.iter_mut() {
                        if src.slug == ext {
                            src.slug = a.slug.clone();
                        }
                    }
                }
            }
        }
        let le = &mut files.lock_edits;
        le.drop.remove(&a.slug);
        le.add.insert(a.slug.clone(), a.object.clone());
        le.wires.extend(a.extern_wires.iter().cloned());
        le.unwires.extend(a.unwires.iter().cloned());
        le.unexternals.extend(a.unexternals.iter().cloned());
    }
    files.repin();
    Ok(Applied {
        previous: op_value(&Op::Unadopt {
            uuids: uuids.to_vec(),
        }),
        added: None,
        warnings: Vec::new(),
    })
}

/// Stop managing blocks adopted here: their declarations and wires leave
/// vislox's file, their lockfile entries and the wires lox tracks for them
/// leave the lock, and Loxone Config owns them again. Only a block whose
/// pending state is the base's: what was changed here goes first.
fn unadopt(files: &mut Files, ctx: &Ctx, uuids: &[String]) -> Result<Applied, String> {
    let name = |u: &str| crate::adopt::name_in(ctx.doc, u);
    let mut picks = Vec::new();
    let mut slugs: BTreeSet<String> = BTreeSet::new();
    for u in uuids {
        let s = ctx
            .lock
            .slug_of(u)
            .ok_or_else(|| format!("{}: the module does not manage it", name(u)))?;
        match files.block(s) {
            Some((_, b)) if b.adopted => {}
            Some(_) => {
                return Err(format!(
                    "{} was added here, not adopted: Remove takes it out",
                    name(u)
                ));
            }
            None => {
                return Err(format!(
                    "{}: the module manages it in its own .loxl file; vislox hands back only a \
                     block it adopted",
                    name(u)
                ));
            }
        }
        if !crate::adopt::same_block(ctx.base, ctx.doc, u) {
            return Err(format!(
                "{} has changes here that the base does not have (values, wires or its place): \
                 revert them first, then hand it back to Loxone Config",
                name(u)
            ));
        }
        if let Some((what, _)) = files.claims.claim_on(s) {
            return Err(format!(
                "{}: vislox's file claims {what}; revert them first, then hand it back to Loxone \
                 Config",
                name(u)
            ));
        }
        slugs.insert(s.to_string());
        picks.push(crate::adopt::Pick {
            uuid: u.clone(),
            slug: Some(s.to_string()),
        });
    }
    let by = |s: &str| {
        ctx.lock
            .uuid_of(s)
            .map(name)
            .unwrap_or_else(|| s.to_string())
    };
    // A wire drawn here (#61) at it would have to change hands too: taken out first.
    if let Some(w) = files
        .claims
        .1
        .iter()
        .find(|w| slugs.contains(&w.to.0) || slugs.contains(&w.from.0))
    {
        let s = if slugs.contains(&w.to.0) {
            &w.to.0
        } else {
            &w.from.0
        };
        return Err(format!(
            "{}: a wire drawn here ends at it ({}); take it out first",
            by(s),
            w.render()
        ));
    }
    // The blocks that stay adopted keep their wires to it: it is an object of
    // Loxone Config's again, so they name it as vislox's extern (`vx_<uuid>`).
    // A wire into it its own declaration bound from such a block becomes that
    // block's wire onto the extern; the rest of its wires are Loxone Config's.
    let mut moved: Vec<(String, blocks::FileWire)> = Vec::new();
    let mut claimed: Vec<Wire> = Vec::new();
    for s in &slugs {
        let (_, b) = files.block(s).expect("checked");
        let ext = slug(ctx.lock.uuid_of(s).expect("checked"));
        for (port, src) in &b.wires {
            if slugs.contains(&src.slug) {
                continue;
            }
            match files.block(&src.slug) {
                Some((_, from)) if from.adopted => moved.push((
                    src.slug.clone(),
                    blocks::FileWire {
                        to: logic::PortRef {
                            slug: ext.clone(),
                            port: port.clone(),
                        },
                        from: src.clone(),
                    },
                )),
                Some(_) if src.port.is_empty() => {
                    return Err(format!(
                        "{} is fed by {}, a block added here, from its default output: vislox \
                         cannot write that wire on its own yet, so it keeps managing it",
                        by(s),
                        by(&src.slug)
                    ));
                }
                Some(_) => claimed.push(Wire {
                    to: (ext.clone(), port.clone()),
                    from: (src.slug.clone(), src.port.clone()),
                }),
                None => {}
            }
        }
    }
    let externs: Vec<(String, String, String)> = slugs
        .iter()
        .map(|s| {
            let u = ctx.lock.uuid_of(s).expect("checked").to_string();
            (s.clone(), slug(&u), u)
        })
        .collect();
    for (s, ext, _) in &externs {
        for p in files.pages.values_mut() {
            for b in p.blocks.iter_mut().filter(|b| !slugs.contains(&b.slug)) {
                for (_, src) in b.wires.iter_mut() {
                    if &src.slug == s {
                        src.slug = ext.clone();
                    }
                }
            }
            for w in p.wires.iter_mut() {
                if &w.to.slug == s && !slugs.contains(&w.from.slug) {
                    w.to.slug = ext.clone();
                }
            }
        }
    }
    for (s, _, u) in &externs {
        let ty = files
            .block(s)
            .map(|(_, b)| b.ty.clone())
            .unwrap_or_default();
        let c = files.claims.0.entry(u.clone()).or_default();
        if c.ty.is_empty() {
            c.ty = ty;
        }
    }
    for s in &slugs {
        for p in files.pages.values_mut() {
            p.blocks.retain(|b| &b.slug != s);
            p.wires.retain(|w| &w.from.slug != s);
        }
        let le = &mut files.lock_edits;
        if let Some(o) = le.add.remove(s) {
            let ports = crate::adopt::ports_of(&o);
            le.wires
                .retain(|w| !ports.contains(w.get("from").and_then(|f| f.as_str()).unwrap_or("")));
        } else {
            le.drop.insert(s.clone());
        }
    }
    for (from, w) in moved {
        if let Some(p) = files
            .pages
            .values_mut()
            .find(|p| p.blocks.iter().any(|b| b.slug == from))
        {
            p.wires.push(w);
        }
    }
    files.claims.1.extend(claimed);
    files.repin();
    Ok(Applied {
        previous: op_value(&Op::Adopt { blocks: picks }),
        added: None,
        warnings: Vec::new(),
    })
}

// ── moving blocks (#60) ─────────────────────────────────────────────────

/// The farthest a block is moved from the page's origin, in twips: well
/// past any sheet Loxone Config draws (A0 is about 67 000 wide).
const MAX_COORD: i64 = 1_000_000;

/// `v` on Loxone Config's grid: it snaps positions to multiples of
/// [`GRID`] on save.
pub fn snap(v: i64) -> i64 {
    (v + GRID / 2).div_euclid(GRID) * GRID
}

/// The `move` that puts the block `uuid` back where `doc` draws it.
fn move_back(doc: &LoxoneDoc, uuid: &str) -> Option<Op> {
    let at = Layout::of_element(find(&doc.xml.root, uuid)?)?;
    Some(Op::Move {
        uuid: uuid.to_string(),
        x: at.px,
        y: at.py,
    })
}

/// Where `base` draws the managed block `slug`, on the page the lock pins it
/// to: the rectangle lox takes over unless it is superseded.
fn base_rect(base: &LoxoneDoc, lock: &Lock, slug: &str) -> Option<Layout> {
    let (page, at) = drawn_at(base, lock.uuid_of(slug)?)?;
    lock.placed
        .get(slug)?
        .as_deref()
        .is_none_or(|p| p == page)
        .then_some(at)
}

/// Move a block the module manages. lox writes the position of the blocks
/// it manages only; one Loxone Config drew is moved there (or adopted
/// first, #72).
fn move_block(files: &mut Files, ctx: &Ctx, uuid: &str, x: i64, y: i64) -> Result<Applied, String> {
    let el = find(&ctx.doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let name = el.attr_decoded("Title").unwrap_or_default().into_owned();
    let Some(slug) = ctx.lock.slug_of(uuid) else {
        return Err(format!(
            "{name}: lox moves only the blocks the module manages, and this one was drawn in \
             Loxone Config; move it there"
        ));
    };
    let (Some(now), true) = (Layout::of_element(el), ctx.lock.placed.contains_key(slug)) else {
        return Err(format!("{name} is not drawn on a page"));
    };
    if x.abs() > MAX_COORD || y.abs() > MAX_COORD {
        return Err(format!("{name}: that is too far off the page"));
    }
    // Within a grid step of where the base draws it, lox keeps the base's
    // rectangle as it is: that is where it goes (Revert, on the grid or
    // not). Anywhere else: on the grid, and not past the page's top left.
    let want = Layout::at(x, y, now.w(), now.h());
    let at = match base_rect(ctx.base, ctx.lock, slug) {
        Some(b) if b.same_spot(&want) => b,
        _ => Layout::at(snap(x).max(0), snap(y).max(0), now.w(), now.h()),
    };
    files.layout.moves.insert(slug.to_string(), at);
    Ok(Applied {
        previous: op_value(&Op::Move {
            uuid: uuid.to_string(),
            x: now.px,
            y: now.py,
        }),
        added: None,
        warnings: Vec::new(),
    })
}

/// Move several managed blocks, each as [`move_block`] does.
fn place(files: &mut Files, ctx: &Ctx, moves: &[Placed]) -> Result<Applied, String> {
    if moves.is_empty() {
        return Err("place: no blocks to move".into());
    }
    let mut back = Vec::new();
    for m in moves {
        let a = move_block(files, ctx, &m.uuid, m.x, m.y)?;
        match serde_json::from_value(a.previous) {
            Ok(Op::Move { uuid, x, y }) => back.push(Placed { uuid, x, y }),
            _ => unreachable!("a move's previous is a move"),
        }
    }
    Ok(Applied {
        previous: op_value(&Op::Place { moves: back }),
        added: None,
        warnings: Vec::new(),
    })
}

/// An op as the reply carries it. An op always serializes: never Null.
pub(crate) fn op_value(op: &Op) -> serde_json::Value {
    serde_json::to_value(op).expect("an op serializes")
}

/// The title lox lays the page `page` out by (`--relayout <title>`), or why
/// it cannot: the base has no such page, another page has its title (lox
/// would lay out both), or the lock pins no block `doc` draws there to it.
pub fn relayout_title(
    base: &LoxoneDoc,
    doc: &LoxoneDoc,
    lock: &Lock,
    page: &str,
) -> Result<String, String> {
    let pg = find(&base.xml.root, page)
        .filter(|e| e.attr("Type") == Some("Page"))
        .ok_or_else(|| format!("no page {page} in the project's base config"))?;
    let title = pg.attr_decoded("Title").unwrap_or_default().into_owned();
    let mut same = Vec::new();
    pages_titled(&base.xml.root, &title, &mut same);
    if title.trim().is_empty() || same.len() != 1 {
        return Err(format!(
            "lox lays out a page by its title, and {} pages are titled \"{title}\"; rename one in \
             Loxone Config first",
            same.len()
        ));
    }
    let drawn_here = |slug: &str| {
        lock.uuid_of(slug)
            .and_then(|u| drawn_at(doc, u))
            .is_some_and(|(q, _)| q == page)
    };
    let pinned = |p: Option<&str>| {
        lock.placed
            .iter()
            .any(|(s, q)| q.as_deref() == p && drawn_here(s))
    };
    if pinned(Some(page)) {
        Ok(title)
    } else if pinned(None) {
        Err(format!(
            "{title}: the lockfile does not say which page its blocks are on (an older lox wrote \
             it); compile the module once with lox logic compile"
        ))
    } else {
        Err(format!(
            "{title}: the module manages no block on this page, and lox lays out only those"
        ))
    }
}

/// Lay out the page's managed blocks afresh, where their wires want them.
/// lox moves only the blocks it manages; one wired to a reference the
/// module does not manage stays beside it.
fn relayout(files: &mut Files, ctx: &Ctx, page: &str) -> Result<Applied, String> {
    let title = relayout_title(ctx.base, ctx.doc, ctx.lock, page)?;
    let back: Vec<Placed> = ctx
        .lock
        .placed
        .iter()
        .filter(|(_, p)| p.as_deref() == Some(page))
        .filter_map(|(s, _)| match move_back(ctx.doc, ctx.lock.uuid_of(s)?)? {
            Op::Move { uuid, x, y } => Some(Placed { uuid, x, y }),
            _ => None,
        })
        .collect();
    // Its undo puts these back; with none, there would be nothing to undo.
    if back.is_empty() {
        return Err(format!(
            "{title}: the pending config draws none of the blocks the module manages on this page"
        ));
    }
    files.layout.relayout.insert(page.to_string(), title);
    Ok(Applied {
        previous: op_value(&Op::Place { moves: back }),
        added: None,
        warnings: Vec::new(),
    })
}

/// The page `uuid` is drawn on in `doc`, and its rectangle there.
fn drawn_at(doc: &LoxoneDoc, uuid: &str) -> Option<(String, Layout)> {
    fn walk(el: &Element, page: Option<&str>, uuid: &str) -> Option<(String, Layout)> {
        let page = if el.attr("Type") == Some("Page") {
            el.attr("U")
        } else {
            page
        };
        if el.attr("U") == Some(uuid) {
            return Some((page?.to_string(), Layout::of_element(el)?));
        }
        el.child_elements().find_map(|c| walk(c, page, uuid))
    }
    if uuid.is_empty() {
        return None;
    }
    walk(&doc.xml.root, None, uuid)
}

/// Every object `doc` draws on a page, by UUID: its page and rectangle.
pub fn drawn(doc: &LoxoneDoc) -> HashMap<String, (String, Layout)> {
    fn walk(el: &Element, page: Option<&str>, out: &mut HashMap<String, (String, Layout)>) {
        let page = if el.attr("Type") == Some("Page") {
            el.attr("U")
        } else {
            page
        };
        if let (Some(p), Some(u), Some(at)) = (page, el.attr("U"), Layout::of_element(el)) {
            out.insert(u.to_string(), (p.to_string(), at));
        }
        for c in el.child_elements() {
            walk(c, page, out);
        }
    }
    let mut out = HashMap::new();
    walk(&doc.xml.root, None, &mut out);
    out
}

/// The lockfile `lock` with `moves` written in, the way lox's own placement
/// leaves a block it moved (`--relayout`): the new rectangle (`layout`),
/// and the one the base still draws (`superseded_layout`). Without the
/// latter, the compile would take the base's position back as a move made
/// in Loxone Config. Once the base draws the block at its new place (after
/// Apply), lox drops the line itself.
pub fn write_moves(
    lock: &[u8],
    base: &LoxoneDoc,
    moves: &BTreeMap<String, Layout>,
) -> Result<Vec<u8>, String> {
    let mut v: serde_json::Value =
        serde_json::from_slice(lock).map_err(|e| format!("cannot read the lock: {e}"))?;
    let shown = drawn(base);
    for (slug, at) in moves {
        let obj = v
            .get_mut("objects")
            .and_then(|o| o.get_mut(slug))
            .and_then(|o| o.as_object_mut())
            .ok_or_else(|| format!("the lock has no block {slug}"))?;
        let uuid = obj
            .get("uuid")
            .and_then(|u| u.as_str())
            .filter(|u| !u.is_empty())
            .ok_or_else(|| format!("the lock has no UUID for block {slug}"))?;
        let page = obj.get("page_uuid").and_then(|p| p.as_str());
        let superseded = shown
            .get(uuid)
            .filter(|(p, _)| page.is_none_or(|q| q == p))
            .map(|(_, l)| *l)
            .filter(|l| !l.same_spot(at));
        obj.insert("layout".into(), serde_json::json!(at));
        match superseded {
            Some(l) => obj.insert("superseded_layout".into(), serde_json::json!(l)),
            None => obj.remove("superseded_layout"),
        };
    }
    let mut out = serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?;
    out.push(b'\n');
    Ok(out)
}

/// The moves of `place` that lox did not keep: `out` does not draw the
/// block where it goes (by `lock`'s UUIDs). lox keeps the base's own
/// rectangle for a move within a grid step of it, so `out` drawing it there
/// keeps that move too. A move on a page laid out afresh is not checked:
/// lox places those blocks anew.
pub fn moves_missed(
    out: &LoxoneDoc,
    base: &LoxoneDoc,
    lock: &Lock,
    place: &Placement,
) -> Vec<String> {
    let relaid = |slug: &str| {
        lock.placed
            .get(slug)
            .and_then(|p| p.as_deref())
            .is_some_and(|p| place.relayout.contains_key(p))
    };
    place
        .moves
        .iter()
        .filter(|(slug, _)| !relaid(slug))
        .filter(|(slug, at)| {
            let Some(d) = lock
                .uuid_of(slug)
                .and_then(|u| Layout::of_element(find(&out.xml.root, u)?))
            else {
                return true;
            };
            let base_kept = base_rect(base, lock, slug)
                .is_some_and(|b| b.same_spot(at) && (d.px, d.py) == (b.px, b.py));
            (d.px, d.py) != (at.px, at.py) && !base_kept
        })
        .map(|(s, _)| s.clone())
        .collect()
}

// ── wires (#61) ─────────────────────────────────────────────────────────

/// What a connector carries. Loxone Config wires a digital output to an
/// analog input and back (a value other than 0 is on), but text only to
/// text. `Unknown`: vislox cannot tell, so it does not wire it (lox does not
/// check what an extern's connector carries).
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    Digital,
    Analog,
    Text,
    Unknown,
}

impl Signal {
    fn word(self) -> &'static str {
        match self {
            Signal::Digital => "digital",
            Signal::Analog => "analog",
            Signal::Text => "text",
            Signal::Unknown => "unknown",
        }
    }
}

/// Loxone draws analog connections differently from digital ones; the
/// connector key is a good tell (`AQ`, `AI`, `AQ1`, `TargetPos`, …).
pub fn analog_key(key: &str) -> bool {
    let k = key.as_bytes();
    (k.len() >= 2 && k[0] == b'A' && (k[1] == b'Q' || k[1] == b'I'))
        || matches!(key, "Txt" | "TQ" | "V" | "Pos" | "Value" | "Temp")
}

/// The text connectors of lox's port table: text outputs (`TQ`, `TQ1`,
/// `TQU`, `OutText`), text inputs (`Text`, `inText`, `TTS`) and message
/// texts (`TM`, `TI`), and `Txt`.
fn text_key(key: &str) -> bool {
    key.starts_with("TQ")
        || matches!(
            key,
            "Txt" | "Text" | "inText" | "OutText" | "TM" | "TI" | "TTS"
        )
}

/// What the connector `key` of an object of type `ty` (its element `el`,
/// if the config has it) carries:
/// - text: a text connector of lox's port table, or a reference to text
///   (`Text="true"`);
/// - an object whose value lox knows (a virtual input, a sensor, a memory
///   flag: the table lox mints references from): that value, and the
///   object's `Analog="true"` or a memory flag's `Tp=`;
/// - a reference: `Analog="true"` or an analog key;
/// - a connector of lox's port table: analog by its key, else digital;
/// - anything else: unknown.
pub fn signal(ty: &str, el: Option<&Element>, key: &str) -> Signal {
    let attr = |a: &str| el.and_then(|e| e.attr(a));
    if attr("Text") == Some("true") || text_key(key) {
        return Signal::Text;
    }
    if let Some((_, v)) = connectors::ref_link_type(ty) {
        return match (v, attr("Tp")) {
            (connectors::RefValue::Text, _) | (_, Some("2")) => Signal::Text,
            (connectors::RefValue::Analog, _) | (_, Some("1")) => Signal::Analog,
            _ if attr("Analog") == Some("true") => Signal::Analog,
            _ => Signal::Digital,
        };
    }
    let is_ref = matches!(ty, "InputRef" | "OutputRef" | "OutputRefLM");
    let known = connectors::builtin(ty).is_some_and(|s| s.iter().any(|s| s.key == key));
    if is_ref || known {
        if analog_key(key) || attr("Analog") == Some("true") {
            Signal::Analog
        } else {
            Signal::Digital
        }
    } else {
        Signal::Unknown
    }
}

/// Who draws a wire vislox's file does not, and so who can take it out.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    /// The rest of the module: a wire lox tracks onto an extern
    /// (`extern_wires`), or one into a block the module declares.
    Module,
    /// Loxone Config.
    LoxoneConfig,
    /// The declaration of a block adopted here (#72): drawn in Loxone Config
    /// before, and brought along by the adoption.
    Adopted,
}

impl Owner {
    /// Why a wire it draws is not changed in vislox.
    pub fn why(self) -> &'static str {
        match self {
            Owner::Module => "The project's module draws this wire: change it in its .loxl file.",
            Owner::LoxoneConfig => {
                "Drawn in Loxone Config. lox removes only the wires its source draws: remove it in \
                 Loxone Config."
            }
            Owner::Adopted => {
                "Drawn in Loxone Config and brought along when the block was adopted: the block's \
                 declaration in vislox's file draws it now. vislox does not take such a wire out yet."
            }
        }
    }
}

/// Who draws the wire from the connector `from` into `to` (connector UUIDs),
/// into the object `sink`, that vislox's file does not draw. One rule for
/// `unwire` and the page (`module_wires` in `GET /api/edits`).
fn owner(lock: &Lock, files: &Files, from: &str, to: &str, source: &str, sink: &str) -> Owner {
    let adopted = |u: &str| {
        lock.slug_of(u)
            .and_then(|s| files.block(s))
            .is_some_and(|(_, b)| b.adopted)
    };
    let tracked = lock.wires.contains(&(from.to_string(), to.to_string()));
    if adopted(sink) || (tracked && adopted(source)) {
        return Owner::Adopted;
    }
    let module_block = lock.slug_of(sink).is_some_and(|s| files.block(s).is_none());
    if module_block || tracked {
        Owner::Module
    } else {
        Owner::LoxoneConfig
    }
}

/// The wires of `doc` that `who` draws, by their ends as the page names
/// them.
pub fn module_wires(
    doc: &LoxoneDoc,
    lock: &Lock,
    files: &Files,
    who: Owner,
) -> Vec<([String; 2], [String; 2])> {
    fn walk<'a>(el: &'a Element, out: &mut HashMap<&'a str, (&'a str, &'a str)>) {
        if let Some(u) = el.attr("U") {
            for co in el.child_elements().filter(|c| c.name == "Co") {
                if let (Some(cu), Some(k)) = (co.attr("U"), co.attr("K")) {
                    out.insert(cu, (u, k));
                }
            }
        }
        for c in el.child_elements() {
            walk(c, out);
        }
    }
    let mut at = HashMap::new();
    walk(&doc.xml.root, &mut at);
    let end = |p: &str| at.get(p).map(|(u, k)| [u.to_string(), k.to_string()]);
    doc.wires()
        .iter()
        .filter_map(|w| {
            let (f, t) = (end(&w.from_port)?, end(&w.to_port)?);
            (owner(lock, files, &w.from_port, &w.to_port, &f[0], &t[0]) == who).then_some((f, t))
        })
        .collect()
}

/// Two ends of a wire as the page names them: `[object UUID, connector key]`.
/// In an `add_block`'s `wires`, an empty UUID is the block it adds.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Ends {
    pub from: [String; 2],
    pub to: [String; 2],
}

/// One end of a wire: the object, its connector, and the name lox knows
/// them by.
pub(crate) struct End<'a> {
    /// Empty for the block the same `add_block` adds.
    pub(crate) uuid: String,
    /// The connector (`<Co>`); none on a block not compiled yet.
    pub(crate) co: Option<&'a Element>,
    /// The object's name, for messages.
    pub(crate) name: String,
    pub(crate) ty: String,
    /// The slug the wire names it by: an extern of [`FILE`], or a block
    /// vislox added. `None` for a block the rest of the module manages.
    pub(crate) slug: Option<String>,
    /// A block vislox added: declared in its files, not an extern.
    pub(crate) added: bool,
    /// The port's Lox Logic name (`In1` for `I1`).
    pub(crate) port: String,
    /// Whether the end is an input (a sink), by lox's port table, else by
    /// the side of the block Loxone Config draws it on. `None`: neither.
    pub(crate) input: Option<bool>,
    pub(crate) signal: Signal,
}

impl End<'_> {
    fn label(&self) -> String {
        format!("{}.{}", self.name, self.port)
    }
}

/// The page geometries an op looked at, by page UUID: built once per page.
pub(crate) type Geo = HashMap<String, Option<loxone_config::layout::PageGeometry>>;

/// Whether the connector `key` (`co`) of the object `uuid` is an input: by
/// lox's port table, else by the side of the block its page draws it on
/// (inputs on the left), else by whether it holds a constant or a wire.
fn port_input(
    doc: &LoxoneDoc,
    ty: &str,
    uuid: &str,
    key: &str,
    co: Option<&Element>,
    geo: &mut Geo,
) -> Option<bool> {
    if let Some(spec) = connectors::builtin(ty).and_then(|s| s.iter().find(|s| s.key == key)) {
        return match spec.dir {
            PortDir::Input | PortDir::Param => Some(true),
            PortDir::Output => Some(false),
            PortDir::Api => None,
        };
    }
    let co = co?;
    let side = drawn_at(doc, uuid).and_then(|(page, _)| {
        let g = geo
            .entry(page.clone())
            .or_insert_with(|| loxone_config::layout::PageGeometry::of(doc, &page))
            .as_ref()?;
        let o = g.objects.iter().find(|o| o.uuid == uuid)?;
        o.ports.iter().find(|p| p.key == key).map(|p| p.side)
    });
    match side {
        Some(loxone_config::layout::Side::Left) => Some(true),
        Some(loxone_config::layout::Side::Right) => Some(false),
        None => (co.attr("Def").is_some() || co.child_elements().any(|c| c.name == "In"))
            .then_some(true),
    }
}

/// The end `[uuid, key]` of a wire in the pending config.
pub(crate) fn end<'a>(
    files: &Files,
    ctx: &'a Ctx,
    at: &[String; 2],
    geo: &mut Geo,
) -> Result<End<'a>, String> {
    let [uuid, key] = at;
    let el = find(&ctx.doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let ty = el.attr("Type").unwrap_or_default().to_string();
    let name = el
        .attr_decoded("Title")
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.into_owned())
        .unwrap_or_else(|| ty.clone());
    let co = el
        .child_elements()
        .find(|c| c.name == "Co" && c.attr("K") == Some(key.as_str()))
        .ok_or_else(|| format!("{name} has no connector {key}"))?;
    let (slug, added) = match ctx.lock.slug_of(uuid) {
        Some(s) if files.block(s).is_some() => (Some(s.to_string()), true),
        Some(_) => (None, false),
        None => (Some(slug(uuid)), false),
    };
    Ok(End {
        input: port_input(ctx.doc, &ty, uuid, key, Some(co), geo),
        signal: signal(&ty, Some(el), key),
        port: port_names::name_of(&ty, key).to_string(),
        uuid: uuid.clone(),
        co: Some(co),
        name,
        ty,
        slug,
        added,
    })
}

/// The end `key` of the block `b` an `add_block` adds now: not in the
/// pending config yet, so known by lox's port table alone.
fn new_end<'a>(b: &NewBlock, key: &str) -> Result<End<'a>, String> {
    let name = b.title.clone().unwrap_or_else(|| b.slug.clone());
    if !connectors::builtin(&b.ty).is_some_and(|s| s.iter().any(|s| s.key == key)) {
        return Err(format!("{name} has no connector {key}"));
    }
    let input = connectors::builtin(&b.ty)
        .and_then(|s| s.iter().find(|s| s.key == key))
        .and_then(|s| match s.dir {
            PortDir::Input | PortDir::Param => Some(true),
            PortDir::Output => Some(false),
            PortDir::Api => None,
        });
    Ok(End {
        uuid: String::new(),
        co: None,
        signal: signal(&b.ty, None, key),
        port: port_names::name_of(&b.ty, key).to_string(),
        name,
        ty: b.ty.clone(),
        slug: Some(b.slug.clone()),
        added: true,
        input,
    })
}

/// The wire `to <- from` as vislox's file names it, if both ends have a slug.
fn wire_of(from: &End, to: &End) -> Option<Wire> {
    Some(Wire {
        to: (to.slug.clone()?, to.port.clone()),
        from: (from.slug.clone()?, from.port.clone()),
    })
}

/// Whether the pending config wires `from` into `to`.
fn wired(from: &End, to: &End) -> bool {
    let (Some(src), Some(co)) = (from.co.and_then(|c| c.attr("U")), to.co) else {
        return false;
    };
    co.child_elements()
        .any(|c| c.name == "In" && c.attr("Input") == Some(src))
}

/// Whether `w` is wired now, as the ops of this request so far leave it:
/// in vislox's file, or in the pending config and not one of vislox's
/// wires this request took out.
fn wired_now(files: &Files, ctx: &Ctx, w: &Wire, src: &End, dst: &End) -> bool {
    files.claims.1.contains(w) || (wired(src, dst) && !ctx.drawn.contains(w))
}

/// Draw a wire from an output to an input. Each end is an object Loxone
/// Config drew (an extern, pinned by UUID) or a block added here; a block
/// the rest of the module manages is wired in its own `.loxl`.
fn wire(
    files: &mut Files,
    ctx: &Ctx,
    from: &[String; 2],
    to: &[String; 2],
) -> Result<Applied, String> {
    let mut geo = Geo::new();
    let (src, dst) = (
        end(files, ctx, from, &mut geo)?,
        end(files, ctx, to, &mut geo)?,
    );
    connect(files, ctx, &src, &dst)?;
    Ok(Applied {
        previous: op_value(&Op::Unwire {
            from: from.clone(),
            to: to.clone(),
        }),
        added: None,
        warnings: Vec::new(),
    })
}

/// Check the wire `src` → `dst` and write it into vislox's file.
pub(crate) fn connect(files: &mut Files, ctx: &Ctx, src: &End, dst: &End) -> Result<(), String> {
    if src.slug.is_some() && src.slug == dst.slug {
        return Err(format!(
            "{}: a wire goes from one block's output to another block's input",
            src.name
        ));
    }
    match src.input {
        Some(false) => {}
        Some(true) => {
            return Err(format!(
                "{} is an input: a wire starts at an output (on a block's right side)",
                src.label()
            ));
        }
        None => return Err(format!("{} cannot be wired", src.label())),
    }
    match dst.input {
        Some(true) => {}
        Some(false) => {
            return Err(format!(
                "{} is an output: a wire ends at an input (on a block's left side)",
                dst.label()
            ));
        }
        None => return Err(format!("{} cannot be wired", dst.label())),
    }
    for e in [src, dst] {
        if e.slug.is_none() {
            return Err(format!(
                "{}: the module manages this block; wire it in its .loxl file",
                e.name
            ));
        }
        // lox matches an extern in the base it compiles against.
        if !e.added && find(&ctx.base.xml.root, &e.uuid).is_none() {
            return Err(format!("{} is not in the project's base config", e.name));
        }
    }
    if dst.co.is_some_and(|c| c.attr("Inv").is_some()) {
        return Err(format!(
            "{} is inverted in Loxone Config, which owns that input; lox does not wire it",
            dst.label()
        ));
    }
    for e in [src, dst] {
        if e.signal == Signal::Unknown {
            return Err(format!(
                "vislox cannot tell whether {} carries text, a number or on/off (lox's port table \
                 does not list it); wire it in Loxone Config",
                e.label()
            ));
        }
    }
    if (src.signal == Signal::Text) != (dst.signal == Signal::Text) {
        return Err(format!(
            "{} is a {} output and {} a {} input: Loxone Config wires text only to text",
            src.label(),
            src.signal.word(),
            dst.label(),
            dst.signal.word()
        ));
    }
    let w = wire_of(src, dst).expect("both have a slug");
    if wired_now(files, ctx, &w, src, dst) {
        return Err(format!(
            "{} is already wired to {}",
            src.label(),
            dst.label()
        ));
    }
    // An input takes a wire or a constant, not both (lox refuses both in
    // one declaration; on an extern the constant would be hidden).
    let constant = match files.block(&w.to.0) {
        Some((_, b)) => b.get(&w.to.1).is_some(),
        None => files
            .claims
            .0
            .get(&dst.uuid)
            .is_some_and(|c| c.sets.contains_key(&w.to.1)),
    };
    if constant {
        return Err(format!(
            "{} has a constant edited here; clear it first, then wire the input",
            dst.label()
        ));
    }
    for e in [src, dst] {
        if !e.added {
            files.claims.0.entry(e.uuid.clone()).or_default().ty = e.ty.clone();
        }
    }
    files.claims.1.insert(w);
    Ok(())
}

/// Take out a wire vislox's file draws. lox removes only the wires its
/// source draws (and tracks in the lockfile): one Loxone Config drew stays.
fn unwire(
    files: &mut Files,
    ctx: &Ctx,
    from: &[String; 2],
    to: &[String; 2],
) -> Result<Applied, String> {
    let mut geo = Geo::new();
    let (src, dst) = (
        end(files, ctx, from, &mut geo)?,
        end(files, ctx, to, &mut geo)?,
    );
    let w = wire_of(&src, &dst);
    let mine = w.as_ref().filter(|w| files.claims.1.contains(w));
    let Some(w) = mine.cloned() else {
        let taken = w.as_ref().is_some_and(|w| ctx.drawn.contains(w));
        if taken || !wired(&src, &dst) {
            return Err(format!("{} is not wired to {}", src.label(), dst.label()));
        }
        let port = |e: &End| {
            e.co.and_then(|c| c.attr("U"))
                .unwrap_or_default()
                .to_string()
        };
        let who = owner(
            ctx.lock,
            files,
            &port(&src),
            &port(&dst),
            &src.uuid,
            &dst.uuid,
        );
        return Err(format!("{} → {}: {}", src.label(), dst.label(), who.why()));
    };
    files.claims.1.remove(&w);
    files.claims.prune();
    Ok(Applied {
        previous: op_value(&Op::Wire {
            from: from.clone(),
            to: to.clone(),
        }),
        added: None,
        warnings: Vec::new(),
    })
}

/// The end `(slug, port name)` of a wire of vislox's files as the page
/// names it, `[object UUID, connector key]`; `None` when the lock does not
/// know the block (yet).
pub(crate) fn wire_end_at(
    files: &Files,
    lock: &Lock,
    (s, port): &(String, String),
) -> Option<[String; 2]> {
    let (uuid, ty) = match files.block(s) {
        Some((_, b)) => (lock.uuid_of(s)?.to_string(), b.ty.clone()),
        // A named signal (#82): its top block's type, as lox pinned it.
        None if files.signal(s).is_some() => {
            let ty = lock.entry(s)?.get("type")?.as_str()?.to_string();
            (lock.uuid_of(s)?.to_string(), ty)
        }
        None => {
            let (u, c) = files.claims.0.iter().find(|(u, _)| slug(u) == *s)?;
            (u.clone(), c.ty.clone())
        }
    };
    Some([uuid, port_names::key_or_raw(&ty, port).to_string()])
}

/// The wires of vislox's files, by their ends as the page names them
/// (`[object UUID, connector key]`): from, to. An end the lock no longer
/// knows is left out.
pub fn wire_ends(files: &Files, lock: &Lock) -> Vec<([String; 2], [String; 2])> {
    files
        .claims
        .1
        .iter()
        .filter_map(|w| {
            Some((
                wire_end_at(files, lock, &w.from)?,
                wire_end_at(files, lock, &w.to)?,
            ))
        })
        .collect()
}

/// A value of a block vislox added: its name, an unwired input's constant,
/// or a setting lox takes in the declaration (`Formula:`, `Type:`, `At:`, …).
/// lox has no room or category for a managed block.
fn edit_new_block(
    files: &mut Files,
    ctx: &Ctx,
    slug: &str,
    op: &Op,
) -> Result<Option<String>, String> {
    let uuid = op.uuid().unwrap_or_default();
    let el = find(&ctx.doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let name = el.attr_decoded("Title").unwrap_or_default().into_owned();
    let caps = caps(el).unwrap_or_default();
    // A circuit of a controller whose moods vislox claims (#74): the claimed
    // set writes the circuit's values anew, so they must fit its new kind.
    if let Op::Setting {
        name: setting,
        value,
        ..
    } = op
        && let Some(n) = connectors::circuit_param(el.attr("Type").unwrap_or(""), setting)
        && let Some(set) = files.claims.3.claims.get(slug)
        && let Some(lines) = files.claims.3.sets.get(set)
    {
        crate::mood_edit::circuit_against_set(
            el,
            find(&ctx.base.xml.root, uuid),
            n,
            value.as_deref().map(str::trim),
            set,
            lines,
        )
        .map_err(|e| format!("{name}: {e}"))?;
    }
    // An operating mode of vislox's (#86) keeps a name no other mode has.
    if let Op::Title { value, .. } = op
        && files
            .block(slug)
            .is_some_and(|(_, b)| b.ty == crate::calendar_edit::MODE)
    {
        let t = value
            .as_deref()
            .ok_or_else(|| format!("{name}: an operating mode keeps a name"))?;
        if let Some(e) = crate::calendar_edit::mode_title_taken(files, ctx, t, Some(slug)) {
            return Err(format!("{t}: {e}"));
        }
    }
    let lets = files.claims.8.clone();
    let b = files.block_mut(slug).expect("checked");
    match op {
        Op::Title { value, .. } => {
            let previous = b.title.clone();
            b.title = value
                .as_deref()
                .map(|t| block_title(&b.ty, t))
                .transpose()?;
            Ok(previous)
        }
        Op::Param { port, value, .. } => {
            let pname = blocks::port_name(&b.ty, port)
                .ok_or_else(|| format!("{name}: {port} cannot be edited here"))?;
            let previous = b.get(pname).map(plain);
            let v = match value {
                Some(v) if caps.params.iter().any(|p| &p.key == port) => {
                    Some(match constant_ref(&lets, ctx.lets, v) {
                        Some(c) => c,
                        None => crate::units::port_value(&b.ty, port, v)
                            .map_err(|e| format!("{pname}: {e}"))?,
                    })
                }
                Some(_) => {
                    return Err(format!(
                        "{name}: {pname} cannot be edited here (it is wired, or not an input)"
                    ));
                }
                None => None,
            };
            b.set(pname, v);
            Ok(previous)
        }
        Op::Setting {
            name: setting,
            value,
            ..
        } => {
            // A lighting controller's circuit (#74): `Circuit<n>: "dimmer"`,
            // which lox's table of settings does not list.
            if connectors::circuit_param(&b.ty, setting).is_some() {
                let previous = b.get(setting).map(plain);
                let v = match value.as_deref().map(str::trim) {
                    Some(v) => {
                        crate::mood_edit::circuit_setting(el, setting, v)
                            .map_err(|e| format!("{name}: {e}"))?;
                        Some(Value::Str(v.to_string()))
                    }
                    None => None,
                };
                b.set(setting, v);
                return Ok(previous);
            }
            let kind = blocks::settings(&b.ty)
                .into_iter()
                .find(|(n, _)| n == setting)
                .map(|(_, k)| k)
                .ok_or_else(|| format!("{name}: {setting} cannot be edited here"))?;
            let previous = b.get(setting).map(plain);
            let mut v = value
                .as_deref()
                .map(|v| setting_value(&b.ty, setting, kind, v))
                .transpose()?;
            // A program (#79) keeps the file it is in, else gets one named
            // after the block.
            if let Some(Value::File { path, .. }) = &mut v {
                *path = match b.get(setting) {
                    Some(Value::File { path: p, .. }) => p.clone(),
                    _ => program_file(slug),
                };
            }
            // An object of the object tree (#84) keeps the settings lox
            // makes it with, and none a Loxone Config save would rewrite.
            if crate::periphery::of(&b.ty).is_some() {
                let mut t = b.clone();
                t.set(setting, v.clone());
                if crate::periphery::missing(&t.ty, &t.args).contains(&setting.as_str()) {
                    return Err(format!(
                        "{name}: a {} needs {setting}; change it, it cannot go",
                        t.ty
                    ));
                }
                if let Some(e) = crate::periphery::rewritten(&t.ty, &t.args) {
                    return Err(format!("{name}: {e}"));
                }
            }
            b.set(setting, v);
            Ok(previous)
        }
        // A calendar entry's mode and date (#86).
        Op::EntryMode { value, .. } | Op::EntryDate { value, .. } => {
            crate::calendar_edit::edit(ctx, &name, b, op, value.as_deref())
        }
        _ => Err(format!(
            "{name}: a block added here has no room or category lox can set; \
             only its name, its inputs' constants and its settings are edited"
        )),
    }
}

/// What the module's own files declare (vislox's directory, [`DIR`], left
/// out: what vislox's files declare is read from [`Files`] as the request
/// changes it, #81).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Decls {
    /// Every name (blocks, externs, signals, instances, sets, constants), so
    /// that a new name is none of them.
    pub names: BTreeSet<String>,
    /// Of them, the `let` constants of a value, which a port may be bound to.
    pub lets: BTreeSet<String>,
    /// And the aliases (`let x = y.Q`), which name a port, not a value.
    pub aliases: BTreeSet<String>,
    /// The templates (#83), by name.
    pub templates: BTreeMap<String, logic::TemplateDecl>,
}

impl Decls {
    /// The bare names lox reads as themselves before an object: the
    /// constants and the aliases (an adopted block keeps them, #72).
    pub fn bare(&self) -> BTreeSet<String> {
        self.lets.union(&self.aliases).cloned().collect()
    }
}

/// [`Decls`] of the module's own `.loxl` files. A file that does not read is
/// an error naming it (shown relative to the module).
pub fn module_decls(module: &Path) -> Result<Decls, String> {
    let mut paths = Vec::new();
    if module.is_file() {
        paths.push(module.to_path_buf());
    } else {
        let ours = module.join(DIR);
        module_files(module, &mut |f, _| {
            if f.extension().is_some_and(|x| x == "loxl") && !f.starts_with(&ours) {
                paths.push(f.to_path_buf());
            }
        });
    }
    paths.sort();
    let mut d = Decls::default();
    for f in paths {
        let shown = f
            .strip_prefix(module)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&f).map_err(|e| format!("{shown}: {e}"))?;
        let m = Module::parse_fragment(&text).map_err(|e| format!("{shown}: {e}"))?;
        for i in &m.items {
            match i {
                Item::Block(b) | Item::Instance(b) => d.names.insert(b.slug.clone()),
                Item::Extern(e) => d.names.insert(e.slug.clone()),
                Item::Signal(s) => d.names.insert(s.slug.clone()),
                Item::Template(t) => {
                    d.templates
                        .entry(t.name.clone())
                        .or_insert_with(|| t.clone());
                    false
                }
                // The sets share the namespace (#73 picks a set's name).
                Item::Schedule(s) => d.names.insert(s.name.clone()),
                Item::Tiles(s) => d.names.insert(s.name.clone()),
                Item::Moods(s) => d.names.insert(s.name.clone()),
                Item::Access(s) => d.names.insert(s.name.clone()),
                Item::Let(l) => {
                    if matches!(l.value, Value::Port(_)) {
                        d.aliases.insert(l.name.clone());
                    } else {
                        d.lets.insert(l.name.clone());
                    }
                    d.names.insert(l.name.clone())
                }
                _ => false,
            };
        }
    }
    Ok(d)
}

/// Check the whole module, with vislox's files as written, the way `lox logic
/// check` and `lint` would: syntax, ports, and the documented range of each
/// value vislox claims. Faster than a compile, and its messages name the value.
pub fn check_module(module: &Path) -> Result<(), String> {
    let path = module
        .to_str()
        .ok_or_else(|| "the module's path is not UTF-8".to_string())?;
    let m = loxone_config::cli::read_module(path).map_err(|e| e.to_string())?;
    // As `lox logic check` does: on the expanded, desugared module, where a
    // hardware declaration (#85) is an extern of its object.
    let (desugared, _) = m
        .expand()
        .and_then(|x| x.desugar())
        .map_err(|e| e.to_string())?;
    logic::validate_ports(&desugared).map_err(|e| e.to_string())?;
    let out_of_range: Vec<String> = logic::lint_source(&m)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|f| f.kind == logic::LintKind::OutOfRange && f.slug.starts_with(SLUG_PREFIX))
        .map(|f| f.detail)
        .collect();
    if out_of_range.is_empty() {
        Ok(())
    } else {
        Err(out_of_range.join("\n"))
    }
}

/// A `.Loxone` file's (or backup's) program, parsed.
pub fn parse_doc(bytes: &[u8]) -> Result<LoxoneDoc, String> {
    let xml = crate::source::program_xml(bytes).map_err(|e| e.to_string())?;
    LoxoneDoc::parse(&xml).map_err(|e| e.to_string())
}

/// The value `op` changes, as `doc` has it (`None`: no such object, or the
/// value is not set). An input without `Def=` holds 0.
#[cfg(test)]
pub fn current(doc: &LoxoneDoc, op: &Op) -> Option<String> {
    current_with(doc, op, None)
}

/// [`current`], with the config's modes by number read once (#86): a
/// calendar entry's `CalType=` names one.
pub fn current_with(
    doc: &LoxoneDoc,
    op: &Op,
    numbers: Option<&crate::calendar_edit::Numbers>,
) -> Option<String> {
    let el = find(&doc.xml.root, op.uuid()?)?;
    let ty = el.attr("Type").unwrap_or_default();
    let decoded = |e: &Element, a: &str| e.attr_decoded(a).map(|v| v.into_owned());
    let place = |a: &str| {
        let io = el.child_elements().find(|c| c.name == "IoData")?;
        let id = io.attr(a).filter(|v| !v.is_empty())?;
        decoded(find(&doc.xml.root, id)?, "Title")
    };
    match op {
        Op::Title { .. } => decoded(el, "Title"),
        Op::Room { .. } => place("Pr"),
        Op::Category { .. } => place("Cr"),
        Op::Param { port, .. } => {
            let key = port_names::key_or_raw(ty, port);
            let co = el
                .child_elements()
                .find(|c| c.name == "Co" && c.attr("K") == Some(key))?;
            Some(co.attr("Def").unwrap_or("0").to_string())
        }
        // A block setting may be stored under another name (`At:` is `Sec=`).
        Op::Setting { name, .. } => decoded(el, connectors::attr_name(ty, name)),
        Op::Statistics { .. } => crate::hw_edit::stats_state(el),
        // A calendar entry (#86): its mode by UUID, its date as the source
        // writes it, both read from what the config stores.
        Op::EntryMode { .. } => match numbers {
            Some(n) => crate::calendar_edit::mode_of(n, el),
            None => crate::calendar_edit::mode_of(&crate::calendar_edit::Numbers::of(doc), el),
        },
        Op::EntryDate { .. } => crate::calendar_edit::date_of(el),
        Op::AddBlock { .. }
        | Op::RemoveBlock { .. }
        | Op::RestoreBlock { .. }
        | Op::Rename { .. }
        | Op::Defaults { .. }
        | Op::Tidy
        | Op::Move { .. }
        | Op::Relayout { .. }
        | Op::Place { .. }
        | Op::Wire { .. }
        | Op::Unwire { .. }
        | Op::Adopt { .. }
        | Op::AdoptPage { .. }
        | Op::Unadopt { .. }
        | Op::Schedule { .. }
        | Op::Moods { .. }
        | Op::Members { .. }
        | Op::Access { .. }
        | Op::Recipients { .. }
        | Op::Tiles { .. }
        | Op::Expr { .. }
        | Op::Signal { .. }
        | Op::Instance { .. }
        | Op::Rule { .. }
        | Op::NewRule { .. }
        | Op::NewPage { .. }
        | Op::RemovePage { .. }
        | Op::RestoreRule { .. }
        | Op::Let { .. } => None,
    }
}

/// Every file of the module directory (not hidden ones, as lox skips them),
/// with its modification time.
pub fn module_files(dir: &Path, found: &mut dyn FnMut(&Path, std::time::SystemTime)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if path.is_dir() {
            module_files(&path, found);
        } else if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
            found(&path, t);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use loxone_config::{Lockfile, logic::CompileOptions};

    static NO_LETS: BTreeSet<String> = BTreeSet::new();

    const DIV: &str = "10000006-0000-0060-ffff504f94000001";
    const AND: &str = "10000004-0000-0040-ffff504f94000001";
    const VI: &str = "10000001-0000-0010-ffff504f94000001";
    const REF: &str = "10000003-0000-0030-ffff504f94000001";
    const AI_IN: &str = "20000005-0000-0051-ffff504f94000002";
    const TOUCH: &str = "20000003-0000-0030-ffff504f94000002";

    fn doc(name: &str) -> LoxoneDoc {
        let p = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        LoxoneDoc::parse(&std::fs::read(p).unwrap()).unwrap()
    }

    fn compile(base: &LoxoneDoc, src: &str) -> String {
        let m = Module::parse(src).unwrap();
        logic::validate_ports(&m).unwrap();
        let opts = CompileOptions {
            machine: [0x50, 0x4f, 0x94, 0, 0, 1],
            mint_time_unix: 0,
            page_title: None,
            allow_removals: false,
            accept_version: None,
            relayout: vec![],
        };
        let out = logic::compile(base, &m, &mut Lockfile::default(), &opts).unwrap();
        String::from_utf8(out.to_bytes()).unwrap()
    }

    fn param(uuid: &str, port: &str, v: Option<&str>) -> Op {
        Op::Param {
            uuid: uuid.into(),
            port: port.into(),
            value: v.map(Into::into),
        }
    }

    #[test]
    fn what_can_be_edited() {
        let d = doc("house.Loxone");
        let div = caps(find(&d.xml.root, DIV).unwrap()).unwrap();
        assert!(div.title);
        // Input1 is wired, AQ an output: only the divisor.
        assert_eq!(
            div.params.iter().map(|p| &p.key[..]).collect::<Vec<_>>(),
            ["Input2"]
        );
        let and = caps(find(&d.xml.root, AND).unwrap()).unwrap();
        assert_eq!(
            and.params.iter().map(|p| &p.key[..]).collect::<Vec<_>>(),
            ["I2"]
        );
        // A reference takes its title from what it mirrors.
        assert_eq!(caps(find(&d.xml.root, REF).unwrap()), None);
        // A virtual input: its name only (its settings stay with Loxone Config).
        let vi = caps(find(&d.xml.root, VI).unwrap()).unwrap();
        assert!(vi.title && vi.params.is_empty() && vi.settings.is_empty());

        let h = doc("hardware.Loxone");
        let ai = caps(find(&h.xml.root, AI_IN).unwrap()).unwrap();
        let names: Vec<_> = ai.settings.iter().map(|s| &s.name[..]).collect();
        // Those it has, and those Lox Logic may add; not Unit, which it lacks.
        assert!(
            names.contains(&"MinChange") && names.contains(&"Desc"),
            "{names:?}"
        );
        assert!(!names.contains(&"Unit"), "{names:?}");
        let min = ai.settings.iter().find(|s| s.name == "MinChange").unwrap();
        assert_eq!((min.text, min.value.as_deref()), (false, Some("0.25")));
        let touch = caps(find(&h.xml.root, TOUCH).unwrap()).unwrap();
        let names: Vec<_> = touch.settings.iter().map(|s| &s.name[..]).collect();
        assert_eq!(names, ["Desc", "Installation"]);
    }

    #[test]
    fn ops_are_checked_and_return_what_they_replace() {
        let d = doc("house.Loxone");
        let none = HashSet::new();
        let mut c = Claims::default();
        assert_eq!(
            apply(
                &mut c,
                &d,
                &none,
                &BTreeSet::new(),
                &param(DIV, "Input2", Some("100"))
            ),
            Ok(None)
        );
        assert_eq!(
            apply(
                &mut c,
                &d,
                &none,
                &BTreeSet::new(),
                &param(DIV, "Input2", Some("50"))
            ),
            Ok(Some("100".into()))
        );
        let err = |op: Op| apply(&mut c.clone(), &d, &none, &BTreeSet::new(), &op).unwrap_err();
        assert!(err(param(DIV, "Input2", Some("1,5"))).contains("decimal point"));
        assert!(err(param(DIV, "Input2", Some("abc"))).contains("not a number"));
        assert!(err(param(DIV, "Input1", Some("3"))).contains("wired"));
        assert!(err(param(DIV, "AQ", Some("3"))).contains("cannot be edited"));
        let title = |v: &str| Op::Title {
            uuid: DIV.into(),
            value: Some(v.into()),
        };
        assert!(err(title("  ")).contains("cannot be empty"));
        assert!(err(title("a\nb")).contains("one line"));
        assert!(err(param("nope", "I2", Some("1"))).contains("no object"));
        let managed = HashSet::from([DIV.to_string()]);
        assert!(
            apply(&mut c.clone(), &d, &managed, &BTreeSet::new(), &title("x"))
                .unwrap_err()
                .contains("module manages")
        );
        // Dropping the claim hands the value back; the object leaves the file.
        assert_eq!(
            apply(
                &mut c,
                &d,
                &none,
                &BTreeSet::new(),
                &param(DIV, "Input2", None)
            ),
            Ok(Some("50".into()))
        );
        assert!(c.0.is_empty());

        let h = doc("hardware.Loxone");
        let set = |n: &str, v: &str| Op::Setting {
            uuid: AI_IN.into(),
            name: n.into(),
            value: Some(v.into()),
        };
        let mut c = Claims::default();
        assert!(
            apply(&mut c, &h, &none, &BTreeSet::new(), &set("MinChange", "-1"))
                .unwrap_err()
                .contains("below 0")
        );
        assert!(
            apply(&mut c, &h, &none, &BTreeSet::new(), &set("Unit", "V"))
                .unwrap_err()
                .contains("cannot be edited")
        );
        apply(
            &mut c,
            &h,
            &none,
            &BTreeSet::new(),
            &set("MinChange", "0.5"),
        )
        .unwrap();
        apply(&mut c, &h, &none, &BTreeSet::new(), &set("Desc", "Pegel")).unwrap();
        assert_eq!(c.ops().len(), 2);
        // An optional text clears to empty (lox writes `Desc=""`); a name cannot.
        apply(&mut c, &h, &none, &BTreeSet::new(), &set("Desc", "")).unwrap();
        assert!(c.render().contains(".Desc = \"\""), "{}", c.render());
        assert!(compile(&h, &c.render()).contains(r#"Desc="""#));
        let no_name = Op::Title {
            uuid: AI_IN.into(),
            value: Some(String::new()),
        };
        assert!(
            apply(&mut c, &h, &none, &BTreeSet::new(), &no_name)
                .unwrap_err()
                .contains("cannot be empty")
        );
    }

    #[test]
    fn the_file_round_trips_and_compiles_with_lox() {
        let d = doc("house.Loxone");
        let none = HashSet::new();
        let mut c = Claims::default();
        for op in [
            param(DIV, "Input2", Some("100")),
            Op::Title {
                uuid: DIV.into(),
                value: Some("Teiler \"neu\"".into()),
            },
            param(AND, "I2", Some("1")),
        ] {
            apply(&mut c, &d, &none, &BTreeSet::new(), &op).unwrap();
        }
        let text = c.render();
        assert!(
            text.contains("vx_10000006_0000_0060_ffff504f94000001.In2 = 100"),
            "{text}"
        );
        assert_eq!(Claims::parse(&text).unwrap(), c);
        assert_eq!(Claims::parse(&text).unwrap().ops().len(), 3);
        // lox's own compiler writes exactly these values.
        let out = compile(&d, &text);
        assert!(out.contains(r#"Title="Teiler &quot;neu&quot;""#), "{out}");
        assert!(out.contains(r#"<Co K="Input2" Def="100""#), "{out}");
        assert!(out.contains(r#"<Co K="I2" Def="1""#), "{out}");

        let h = doc("hardware.Loxone");
        let mut c = Claims::default();
        apply(
            &mut c,
            &h,
            &none,
            &BTreeSet::new(),
            &Op::Setting {
                uuid: AI_IN.into(),
                name: "MinChange".into(),
                value: Some("0.5".into()),
            },
        )
        .unwrap();
        assert!(compile(&h, &c.render()).contains(r#"MinChange="0.5""#));
    }

    #[test]
    fn room_and_category_are_claimed_by_title() {
        let xml = std::fs::read_to_string(format!(
            "{}/tests/fixtures/house.Loxone",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
        .replace(
            "\t\t<C Type=\"Page\" V=\"178\" U=\"10000002",
            "\t\t<C Type=\"Place\" U=\"30000a01-0000-0000-ffff504f94000001\" Title=\"Küche\"/>\n\
             \t\t<C Type=\"Place\" U=\"30000a02-0000-0000-ffff504f94000001\" Title=\"Bad\"/>\n\
             \t\t<C Type=\"Place\" U=\"30000a04-0000-0000-ffff504f94000001\" Title=\"Flur\"/>\n\
             \t\t<C Type=\"Place\" U=\"30000a05-0000-0000-ffff504f94000001\" Title=\"Flur\"/>\n\
             \t\t<C Type=\"Category\" U=\"30000a03-0000-0000-ffff504f94000001\" Title=\"Licht\"/>\n\
             \t\t<C Type=\"Page\" V=\"178\" U=\"10000002",
        )
        .replace(
            "<Co K=\"Input1\" Nc=\"1\"",
            "<IoData Visu=\"true\" Pr=\"30000a01-0000-0000-ffff504f94000001\" Cr=\"30000a03-0000-0000-ffff504f94000001\"/><Co K=\"Input1\" Nc=\"1\"",
        );
        let d = LoxoneDoc::parse(xml.as_bytes()).unwrap();
        let div = caps(find(&d.xml.root, DIV).unwrap()).unwrap();
        assert!(div.room && div.category);
        assert!(!caps(find(&d.xml.root, AND).unwrap()).unwrap().room);
        let none = HashSet::new();
        let room = |uuid: &str, v: &str| Op::Room {
            uuid: uuid.into(),
            value: Some(v.into()),
        };
        let mut c = Claims::default();
        let err = |c: &mut Claims, op: Op| apply(c, &d, &none, &BTreeSet::new(), &op).unwrap_err();
        assert!(err(&mut c, room(DIV, "Keller")).contains("no room"));
        assert!(err(&mut c, room(DIV, "Flur")).contains("more than one"));
        assert!(err(&mut c, room(AND, "Bad")).contains("has no room"));
        apply(&mut c, &d, &none, &BTreeSet::new(), &room(DIV, "Bad")).unwrap();
        let text = c.render();
        assert!(text.contains(".room = \"Bad\""), "{text}");
        assert_eq!(Claims::parse(&text).unwrap(), c);
        assert!(compile(&d, &text).contains(r#"Pr="30000a02-0000-0000-ffff504f94000001""#));
    }

    #[test]
    fn current_values_are_read_as_a_config_has_them() {
        let d = doc("house.Loxone");
        let get = |op: Op| current(&d, &op);
        assert_eq!(get(param(DIV, "Input2", None)).as_deref(), Some("1000"));
        // No `Def=`: 0.
        assert_eq!(get(param(AND, "I2", None)).as_deref(), Some("0"));
        let t = Op::Title {
            uuid: DIV.into(),
            value: None,
        };
        assert_eq!(get(t).as_deref(), Some("O1002"));
        let room = Op::Room {
            uuid: DIV.into(),
            value: None,
        };
        assert_eq!(get(room), None);
        assert_eq!(get(param("nope", "I1", None)), None);
    }

    #[test]
    fn statements_vislox_did_not_write_are_refused() {
        assert!(
            Claims::parse("a = And()\n")
                .unwrap_err()
                .contains("did not write")
        );
        assert!(
            Claims::parse("extern x = Div(uuid: \"10000006-0000-0060-ffff504f94000001\")\n")
                .unwrap_err()
                .contains("did not write")
        );
        assert!(Claims::parse("x.title = \"a\"\n").is_err());
        assert_eq!(Claims::parse("# nothing yet\n").unwrap(), Claims::default());
    }

    #[test]
    fn the_module_check_names_a_value_out_of_its_range() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pages");
        std::fs::create_dir_all(dir.join("vislox")).unwrap();
        let uuid = "1000000a-0000-00a0-ffff504f94000001";
        let s = slug(uuid);
        std::fs::write(
            dir.join(FILE),
            format!("extern {s} = Monoflop(uuid: \"{uuid}\")\n{s}.Time = -3\n"),
        )
        .unwrap();
        let e = check_module(&dir).unwrap_err();
        assert!(e.contains("outside the documented range"), "{e}");
        std::fs::write(
            dir.join(FILE),
            format!("extern {s} = Monoflop(uuid: \"{uuid}\")\n{s}.Time = 3\n"),
        )
        .unwrap();
        check_module(&dir).unwrap();
    }

    const EG: &str = "10000002-0000-0020-ffff504f94000001";
    const OG: &str = "10000008-0000-0080-ffff504f94000001";

    /// Compile vislox's files (by name, in `<module>/vislox`) with lox's own
    /// compiler, as `lox logic compile` would, keeping the lock.
    fn compile_files(
        base: &LoxoneDoc,
        files: &Files,
        lock: &mut Lockfile,
    ) -> Result<LoxoneDoc, String> {
        // Written out and read back as lox reads a module: a program
        // (#79) comes from its file.
        let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
        // Not under the temporary directory's own hidden name: lox skips those.
        let module = tmp.path().join("module");
        let dir = module.join(DIR);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut texts = files.all_texts();
        if texts.is_empty() {
            texts.extend([Files::placeholder()]);
        }
        for (name, t) in texts {
            std::fs::write(dir.join(name), t).map_err(|e| e.to_string())?;
        }
        let m =
            loxone_config::cli::read_module(module.to_str().unwrap()).map_err(|e| e.to_string())?;
        // As `lox logic check` does: on the expanded, desugared module, where
        // a hardware declaration (#85) is an extern.
        let (desugared, _) = m
            .expand()
            .and_then(|x| x.desugar())
            .map_err(|e| e.to_string())?;
        logic::validate_ports(&desugared).map_err(|e| e.to_string())?;
        let opts = CompileOptions {
            machine: [0x50, 0x4f, 0x94, 0, 0, 1],
            mint_time_unix: 0,
            page_title: None,
            allow_removals: false,
            accept_version: None,
            relayout: vec![],
        };
        let out = logic::compile(base, &m, lock, &opts).map_err(|e| e.to_string())?;
        LoxoneDoc::parse(&out.to_bytes()).map_err(|e| e.to_string())
    }

    fn lock_of(l: &Lockfile) -> Lock {
        Lock {
            slugs: l
                .objects
                .iter()
                .map(|(s, o)| (s.clone(), o.uuid.clone()))
                .collect(),
            placed: l
                .objects
                .iter()
                .filter(|(_, o)| o.layout.is_some())
                .map(|(s, o)| (s.clone(), o.page_uuid.clone()))
                .collect(),
            wires: l
                .extern_wires
                .iter()
                .map(|w| (w.from.clone(), w.to.clone()))
                .collect(),
            externals: l
                .externals
                .iter()
                .map(|(s, e)| (s.clone(), e.uuid.clone()))
                .collect(),
            entries: l
                .objects
                .iter()
                .map(|(s, o)| (s.clone(), serde_json::to_value(o).unwrap()))
                .collect(),
            tombs: l
                .removed
                .iter()
                .map(|(u, t)| (u.clone(), (t.slug.clone(), t.block_type.clone())))
                .collect(),
            owned: l
                .externals
                .iter()
                .filter(|(_, e)| e.owned)
                .map(|(s, _)| s.clone())
                .collect(),
            rules: l.rules.clone(),
            pages: l.pages.clone(),
        }
    }

    /// A base with a program, rooms and categories, as Loxone Config writes
    /// them: lox creates a page only inside a `<C Type="Program">` (#88).
    pub(crate) const PROGRAM_BASE: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<ControlList Version=\"1\" NextObj=\"50\" NextConst=\"1\" NextNote=\"1\" NextMem=\"1\">\n\
\t<C Type=\"Document\" V=\"17020000\" U=\"20000000-0000-0001-ffff504f94000001\" Title=\"Haus\" ConfigVersion=\"17020000\">\n\
\t\t<C Type=\"CategoryCaption\" V=\"178\" U=\"20000000-0000-0002-ffff504f94000001\" Title=\"Kategorien\">\n\
\t\t\t<C Type=\"Category\" V=\"178\" U=\"20000000-0000-0003-ffff504f94000001\" Title=\"Nicht zugeordnet\" First=\"true\"/>\n\
\t\t\t<C Type=\"Category\" V=\"178\" U=\"20000000-0000-0004-ffff504f94000001\" Title=\"Beleuchtung\"/>\n\
\t\t</C>\n\
\t\t<C Type=\"PlaceCaption\" V=\"178\" U=\"20000000-0000-0005-ffff504f94000001\" Title=\"Räume\">\n\
\t\t\t<C Type=\"Place\" V=\"178\" U=\"20000000-0000-0006-ffff504f94000001\" Title=\"Nicht zugeordnet\" First=\"true\"/>\n\
\t\t\t<C Type=\"Place\" V=\"178\" U=\"20000000-0000-0007-ffff504f94000001\" Title=\"Küche\"/>\n\
\t\t</C>\n\
\t\t<C Type=\"Program\" V=\"178\" U=\"20000000-0000-0008-ffff504f94000001\" Title=\"Loxone Miniserver\">\n\
\t\t\t<C Type=\"Page\" V=\"178\" U=\"20000000-0000-000a-ffff504f94000001\" Title=\"Küche\">\n\
\t\t\t\t<IoData Cr=\"20000000-0000-0003-ffff504f94000001\" Pr=\"20000000-0000-0007-ffff504f94000001\"/>\n\
\t\t\t</C>\n\
\t\t</C>\n\
\t</C>\n\
</ControlList>\n";

    /// A new page with lox's own compiler (#88): the page line alone creates
    /// it in the room and category it names, the lock keeps its UUID, a block
    /// added on it goes into its file, and once empty `remove_page` takes it
    /// out again. Deployed, the line loses `room:`, and the compile keeps the
    /// page as Loxone Config has it.
    #[test]
    fn a_new_page_is_created_by_lox_and_taken_out_again() {
        let base = LoxoneDoc::parse(PROGRAM_BASE.as_bytes()).unwrap();
        let mut lockfile = Lockfile::default();
        let mut files = Files::default();
        let lock = lock_of(&lockfile);
        let none = BTreeSet::new();
        let names = BTreeSet::new();
        let drawn = BTreeSet::new();
        let ad = BTreeMap::new();
        let pad = BTreeMap::new();
        let claimed = Default::default();
        let mk = |doc, lock| Ctx {
            doc,
            base: &base,
            lock,
            deployed: None,
            names: &names,
            lets: &none,
            drawn: &drawn,
            adoptions: &ad,
            page_adoptions: &pad,
            claimed: &claimed,
            templates: &NO_TEMPLATES,
            module: &crate::module_edit::NO_MODULE,
        };
        let op = Op::NewPage {
            title: "Küche Logik".into(),
            room: "Küche".into(),
            category: Some("Beleuchtung".into()),
            restores: None,
        };
        apply_op(&mut files, &mk(&base, &lock), &op).unwrap();
        let out = compile_files(&base, &files, &mut lockfile).unwrap();
        let uuid = lockfile.pages["Küche Logik"].clone();
        let pg = find(&out.xml.root, &uuid).unwrap();
        assert_eq!(pg.attr("Type"), Some("Page"));
        assert_eq!(pg.attr_decoded("Title").as_deref(), Some("Küche Logik"));
        let io = pg.child_elements().find(|c| c.name == "IoData").unwrap();
        assert_eq!(io.attr("Pr"), Some("20000000-0000-0007-ffff504f94000001"));
        assert_eq!(io.attr("Cr"), Some("20000000-0000-0004-ffff504f94000001"));
        // The next edit: a block goes on the page, found by the UUID the lock
        // keeps; the file takes no `# page uuid:` line until it is deployed.
        let lock = lock_of(&lockfile);
        let add = add(&uuid, "And", Some("Neu"), &[], &[]);
        apply_op(&mut files, &mk(&out, &lock), &add).unwrap();
        let text = &files.render()["kueche-logik.loxl"];
        assert!(!text.contains("# page uuid:"), "{text}");
        let out = compile_files(&base, &files, &mut lockfile).unwrap();
        let block = lockfile.objects["vx_and_1"].uuid.clone();
        let lock = lock_of(&lockfile);
        assert_eq!(lock.placed["vx_and_1"].as_deref(), Some(uuid.as_str()));
        assert!(find(&out.xml.root, &block).is_some());
        // Not while the block is there. The undo of both, in one request: the
        // block's `removed` line stays (lox needs it) on a page line that
        // creates no page any more, and the compile takes both out.
        let remove = Op::RemovePage { page: uuid.clone() };
        assert!(apply_op(&mut files.clone(), &mk(&out, &lock), &remove).is_err());
        apply_op(
            &mut files,
            &mk(&out, &lock),
            &Op::RemoveBlock {
                uuid: block.clone(),
            },
        )
        .unwrap();
        let a = apply_op(&mut files, &mk(&out, &lock), &remove).unwrap();
        let text = &files.render()["kueche-logik.loxl"];
        assert!(
            text.contains("page \"Küche Logik\"\n") && text.contains("removed vx_and_1\n"),
            "{text}"
        );
        let out = compile_files(&base, &files, &mut lockfile).unwrap();
        assert!(find(&out.xml.root, &uuid).is_none());
        assert!(find(&out.xml.root, &block).is_none());
        assert!(lockfile.pages.is_empty() && lockfile.objects.is_empty());
        // The next edit drops the line, and the file with it.
        files.tidy(&lock_of(&lockfile));
        assert!(files.render().is_empty());
        // Its undo creates it again (a new UUID: `restores` names the old one).
        let again: Op = serde_json::from_value(a.previous).unwrap();
        assert!(matches!(&again, Op::NewPage { restores: Some(r), .. } if *r == uuid));
        let lock = lock_of(&lockfile);
        apply_op(&mut files, &mk(&out, &lock), &again).unwrap();
        let out = compile_files(&base, &files, &mut lockfile).unwrap();
        let uuid = lockfile.pages["Küche Logik"].clone();
        // Deployed: the base has the page, and Loxone Config renamed it.
        let deployed = String::from_utf8(out.to_bytes())
            .unwrap()
            .replace("Title=\"Küche Logik\"", "Title=\"Kochen\"");
        let deployed = LoxoneDoc::parse(deployed.as_bytes()).unwrap();
        let lock = lock_of(&lockfile);
        let changed = crate::page_edit::hand_off(&mut files, &[&deployed], &lock);
        assert_eq!(changed, ["kueche-logik.loxl"]);
        let p = &files.pages["kueche-logik.loxl"];
        assert_eq!((p.page.as_str(), p.room.as_deref()), ("Kochen", None));
        assert_eq!(p.uuid.as_deref(), Some(uuid.as_str()));
        let mut tmp = files.clone();
        tmp.pages
            .get_mut("kueche-logik.loxl")
            .unwrap()
            .blocks
            .push(blocks::NewBlock {
                slug: "vx_and_1".into(),
                ty: "And".into(),
                ..Default::default()
            });
        let out = compile_files(&deployed, &tmp, &mut lockfile).unwrap();
        let pg = find(&out.xml.root, &uuid).unwrap();
        assert_eq!(pg.attr_decoded("Title").as_deref(), Some("Kochen"));
    }

    fn add(
        page: &str,
        ty: &str,
        title: Option<&str>,
        params: &[(&str, &str)],
        settings: &[(&str, &str)],
    ) -> Op {
        let m = |v: &[(&str, &str)]| {
            v.iter()
                .map(|(k, x)| (k.to_string(), x.to_string()))
                .collect()
        };
        Op::AddBlock {
            page: page.into(),
            ty: ty.into(),
            title: title.map(Into::into),
            params: m(params),
            settings: m(settings),
            wires: vec![],
            restores: None,
        }
    }

    #[test]
    fn the_undo_of_a_request_is_each_previous_last_op_first() {
        let t = |v: Option<&str>| Op::Title {
            uuid: DIV.into(),
            value: v.map(Into::into),
        };
        // A relayout of however many blocks is undone by one `place`: the
        // undo of a request never has more ops than the request.
        let place = Op::Place {
            moves: (0..500)
                .map(|i| Placed {
                    uuid: format!("u{i}"),
                    x: 96 * i,
                    y: 0,
                })
                .collect(),
        };
        let ops = vec![
            t(Some("A")),
            add("p", "And", None, &[], &[]),
            Op::Relayout { page: "p".into() },
            Op::RemoveBlock { uuid: "r".into() },
        ];
        let mut again = add("p", "Or", None, &[], &[]);
        if let Op::AddBlock { restores, .. } = &mut again {
            *restores = Some("r".into());
        }
        let previous = vec![
            serde_json::json!(null),
            serde_json::json!(null),
            serde_json::to_value(place.clone()).unwrap(),
            serde_json::to_value(&again).unwrap(),
        ];
        let minted = [None, Some("n".to_string()), None, None];
        assert_eq!(
            undo_ops(&ops, &previous, &minted).unwrap(),
            vec![again, place, Op::RemoveBlock { uuid: "n".into() }, t(None)]
        );
        assert_eq!(undo_ops(&ops, &previous, &minted).unwrap().len(), ops.len());
        // A block lox minted no UUID for cannot be taken out again; and a
        // previous that is no op says which op.
        let e = undo_ops(&ops, &previous, &[]).unwrap_err();
        assert!(e.contains("op 2 (add_block)"), "{e}");
        let mut broken = previous.clone();
        broken[2] = serde_json::Value::Null;
        let e = undo_ops(&ops, &broken, &minted).unwrap_err();
        assert!(e.contains("op 3 (relayout)"), "{e}");
        // `restores` is echoed, not written: it round-trips, and is left out when none.
        let v = serde_json::to_value(add("p", "And", None, &[], &[])).unwrap();
        assert_eq!(v.get("restores"), None);
    }

    /// The page an object is drawn on.
    fn page_of(doc: &LoxoneDoc, uuid: &str) -> Option<String> {
        fn walk(el: &Element, page: Option<&str>, uuid: &str) -> Option<String> {
            let page = if el.attr("Type") == Some("Page") {
                el.attr("U")
            } else {
                page
            };
            if el.attr("U") == Some(uuid) {
                return page.map(String::from);
            }
            el.child_elements().find_map(|c| walk(c, page, uuid))
        }
        walk(&doc.xml.root, None, uuid)
    }

    #[test]
    fn every_offered_type_is_created_by_lox() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut failed = Vec::new();
        for t in connectors::BUILTIN_TYPES
            .iter()
            .filter(|t| blocks::offered(t))
        {
            let mut files = Files::default();
            let lock = Lock::default();
            let ctx = Ctx {
                doc: &d,
                lock: &lock,
                base: &d,
                deployed: None,
                names: &names,
                lets: &Default::default(),
                drawn: &Default::default(),
                adoptions: &Default::default(),
                page_adoptions: &Default::default(),
                claimed: &Default::default(),
                templates: &Default::default(),
                module: &Default::default(),
            };
            apply_op(&mut files, &ctx, &add(EG, t, None, &[], &[])).unwrap();
            let mut l = Lockfile::default();
            match compile_files(&d, &files, &mut l) {
                Ok(out) => {
                    let uuid = &l.objects.values().next().unwrap().uuid;
                    let el = find(&out.xml.root, uuid).unwrap();
                    assert_eq!(el.attr("Type"), Some(*t));
                    assert_eq!(page_of(&out, uuid).as_deref(), Some(EG), "{t}");
                }
                Err(e) => failed.push(format!("{t}: {e}")),
            }
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }

    #[test]
    fn a_block_is_added_named_and_given_values_then_taken_out() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let a = apply_op(
            &mut files,
            &ctx,
            &add(OG, "Monoflop", Some("Nachlauf"), &[("Time", "30")], &[]),
        )
        .unwrap();
        assert_eq!(
            a,
            Applied {
                previous: serde_json::Value::Null,
                added: Some("vx_monoflop_1".into()),
                warnings: Vec::new(),
            }
        );
        let b = apply_op(
            &mut files,
            &ctx,
            &add(
                OG,
                "PulseAt",
                None,
                &[],
                &[("Pulse", "time"), ("At", "7:05")],
            ),
        )
        .unwrap();
        assert_eq!(b.added.as_deref(), Some("vx_pulseat_1"));
        let text = &files.render()["obergeschoss.loxl"];
        assert!(text.starts_with("# Blocks added in vislox"), "{text}");
        assert!(text.contains("page \"Obergeschoss\"\n"), "{text}");
        assert!(
            text.contains("vx_monoflop_1 = Monoflop(\"Nachlauf\", Time: 30)\n"),
            "{text}"
        );
        assert!(
            text.contains("vx_pulseat_1 = PulseAt(At: 07:05, Pulse: \"time\")\n"),
            "{text}"
        );
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().pages,
            files.pages
        );

        let out = compile_files(&d, &files, &mut l).unwrap();
        let mono = l.objects["vx_monoflop_1"].uuid.clone();
        let el = find(&out.xml.root, &mono).unwrap();
        assert_eq!(el.attr("Title"), Some("Nachlauf"));
        assert_eq!(page_of(&out, &mono).as_deref(), Some(OG));
        let time = el
            .child_elements()
            .find(|c| c.attr("K") == Some("Time"))
            .unwrap();
        assert_eq!(time.attr("Def"), Some("30"));
        let pulse = find(&out.xml.root, &l.objects["vx_pulseat_1"].uuid).unwrap();
        assert_eq!(pulse.attr("Sec"), Some("25500"));

        // Edited like any block, in its declaration; the module's own
        // blocks are not.
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: Some(&d),
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let title = Op::Title {
            uuid: mono.clone(),
            value: Some("Treppe".into()),
        };
        assert_eq!(
            apply_op(&mut files, &ctx, &title).unwrap().previous,
            "Nachlauf"
        );
        let p = param(&mono, "Time", Some("45"));
        assert_eq!(apply_op(&mut files, &ctx, &p).unwrap().previous, "30");
        // A setting is written into the declaration too.
        let pulse = l.objects["vx_pulseat_1"].uuid.clone();
        let at = Op::Setting {
            uuid: pulse.clone(),
            name: "At".into(),
            value: Some("8:00".into()),
        };
        assert_eq!(apply_op(&mut files, &ctx, &at).unwrap().previous, "07:05");
        let bad = Op::Setting {
            uuid: pulse.clone(),
            name: "Pulse".into(),
            value: Some("often".into()),
        };
        assert!(
            apply_op(&mut files, &ctx, &bad)
                .unwrap_err()
                .contains("\"time\"")
        );
        assert!(files.claims.0.is_empty());
        // Title, Time; At, Pulse.
        assert_eq!(files.ops(&lock).len(), 4);
        let room = Op::Room {
            uuid: mono.clone(),
            value: Some("Bad".into()),
        };
        assert!(
            apply_op(&mut files, &ctx, &room)
                .unwrap_err()
                .contains("only its name")
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        let el = find(&out.xml.root, &mono).unwrap();
        assert_eq!(el.attr("Title"), Some("Treppe"));
        let at = Op::Setting {
            uuid: pulse.clone(),
            name: "At".into(),
            value: None,
        };
        assert_eq!(current(&out, &at).as_deref(), Some("28800"));

        // Taken out: `removed` tells lox, and the op that adds it again comes back.
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: Some(&d),
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let r = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: mono.clone() }).unwrap();
        // It names the block it adds again (#62): lox mints new UUIDs for it.
        let mut again = add(OG, "Monoflop", Some("Treppe"), &[("Time", "45")], &[]);
        if let Op::AddBlock { restores, .. } = &mut again {
            *restores = Some(mono.clone());
        }
        assert_eq!(serde_json::from_value::<Op>(r.previous).unwrap(), again);
        assert!(files.render()["obergeschoss.loxl"].contains("removed vx_monoflop_1\n"));
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert!(find(&out.xml.root, &mono).is_none());
        // Once lox has forgotten it, the line goes.
        files.tidy(&lock_of(&l));
        assert!(!files.render()["obergeschoss.loxl"].contains("removed"));
        // A deployed block is deleted (#80): its undo brings it back with its
        // UUIDs, not as a new block.
        let pulse = l.objects["vx_pulseat_1"].uuid.clone();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: Some(&out),
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let r = apply_op(&mut files.clone(), &ctx, &Op::RemoveBlock { uuid: pulse }).unwrap();
        assert_eq!(r.previous["op"], "restore_block");
        let e = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: DIV.into() }).unwrap_err();
        assert!(e.contains("only a block added or adopted here"), "{e}");
    }

    #[test]
    fn a_page_file_follows_its_page_and_goes_with_its_last_block() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            base: &d,
            lock: &lock,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", Some("Licht"), &[], &[])).unwrap();
        let text = files.render()["obergeschoss.loxl"].clone();
        assert!(
            text.contains(&format!("# page uuid: {OG}\npage \"Obergeschoss\"")),
            "{text}"
        );
        compile_files(&d, &files, &mut l).unwrap();
        // Loxone Config renames the page: the same file follows it, by UUID.
        let renamed = LoxoneDoc::parse(
            std::fs::read_to_string(format!(
                "{}/tests/fixtures/house.Loxone",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap()
            .replace("Title=\"Obergeschoss\"", "Title=\"Oben\"")
            .as_bytes(),
        )
        .unwrap();
        let mut files = Files::from_texts(&files.render()).unwrap();
        files.follow_pages(&renamed);
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &renamed,
            base: &renamed,
            lock: &lock,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "Or", None, &[], &[])).unwrap();
        let r = files.render();
        assert_eq!(r.keys().collect::<Vec<_>>(), ["obergeschoss.loxl"]);
        assert!(r["obergeschoss.loxl"].contains("page \"Oben\"\n"), "{r:?}");
        let out = compile_files(&renamed, &files, &mut l).unwrap();
        for s in ["vx_and_1", "vx_or_1"] {
            assert_eq!(
                page_of(&out, &l.objects[s].uuid).as_deref(),
                Some(OG),
                "{s}"
            );
        }
        // A file written before it had a UUID takes the page with its title.
        let mut old = Files::from_texts(&BTreeMap::from([(
            "oben.loxl".to_string(),
            "page \"Oben\"\nvx_and_1 = And()\n".to_string(),
        )]))
        .unwrap();
        old.follow_pages(&renamed);
        assert_eq!(old.pages["oben.loxl"].uuid.as_deref(), Some(OG));

        // Both taken out, and forgotten by lox: the file goes.
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            base: &renamed,
            lock: &lock,
            deployed: Some(&renamed),
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        for s in ["vx_and_1", "vx_or_1"] {
            let uuid = l.objects[s].uuid.clone();
            apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid }).unwrap();
        }
        assert!(files.render().contains_key("obergeschoss.loxl"));
        compile_files(&renamed, &files, &mut l).unwrap();
        files.tidy(&lock_of(&l));
        assert!(files.render().is_empty(), "{:?}", files.render());
    }

    /// A program (#79) is a file of its own next to its block's file, named
    /// by `Code: file(…)`; edited, it keeps the file, and dropped, the file
    /// goes. lox compiles it into the block's `Code=`.
    #[test]
    fn a_program_is_a_file_of_its_own() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let mut files = Files::default();
        let prog = "int main() {\r\n  setoutput(0, 1);\r\n}\r\n";
        apply_op(
            &mut files,
            &ctx,
            &add(EG, "Code16", Some("Hue"), &[], &[("Code", prog)]),
        )
        .unwrap();
        let texts = files.all_texts();
        assert!(
            texts["erdgeschoss.loxl"]
                .contains(r#"vx_code16_1 = Code16("Hue", Code: file("vx_code16_1.c"))"#),
            "{texts:?}"
        );
        assert_eq!(
            texts["vx_code16_1.c"],
            "int main() {\n  setoutput(0, 1);\n}\n"
        );
        // Read back, the block holds the program's text.
        let back = Files::from_texts(&texts).unwrap();
        assert_eq!(back.pages, files.pages);
        let out = compile_files(&d, &files, &mut l).unwrap();
        let uuid = l.objects["vx_code16_1"].uuid.clone();
        let el = find(&out.xml.root, &uuid).unwrap();
        assert_eq!(
            el.attr_decoded("Code").unwrap(),
            "int main() {\n  setoutput(0, 1);\n}\n"
        );
        // Edited: the same file, and `previous` is the old text.
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            base: &out,
            lock: &lock,
            ..ctx
        };
        let setting = |v: Option<&str>| Op::Setting {
            uuid: uuid.clone(),
            name: "Code".into(),
            value: v.map(Into::into),
        };
        let a = apply_op(&mut files, &ctx, &setting(Some("int main() {}\n"))).unwrap();
        assert_eq!(
            a.previous,
            serde_json::json!("int main() {\n  setoutput(0, 1);\n}\n")
        );
        assert_eq!(files.all_texts()["vx_code16_1.c"], "int main() {}\n");
        // The op carries the text, so undo puts it back.
        assert!(files.ops(&lock).contains(&setting(Some("int main() {}\n"))));
        // Dropped: the binding and the file go; Loxone Config's program again.
        let a = apply_op(&mut files, &ctx, &setting(None)).unwrap();
        assert_eq!(a.previous, serde_json::json!("int main() {}\n"));
        let texts = files.all_texts();
        assert!(!texts.contains_key("vx_code16_1.c"));
        assert!(texts["erdgeschoss.loxl"].contains(r#"vx_code16_1 = Code16("Hue")"#));
        // A program no block names, or a block whose program is gone, is
        // refused when the files are read.
        let mut orphan = texts.clone();
        orphan.insert("stray.c".into(), "x".into());
        assert!(Files::from_texts(&orphan).unwrap_err().contains("stray.c"));
        let mut lost = texts.clone();
        lost.insert(
            "erdgeschoss.loxl".into(),
            texts["erdgeschoss.loxl"]
                .replace(r#"Code16("Hue")"#, r#"Code16("Hue", Code: file("gone.c"))"#),
        );
        assert!(Files::from_texts(&lost).unwrap_err().contains("gone.c"));
        // Two blocks naming one program file: refused, saying so.
        let mut shared = texts.clone();
        shared.insert("vx_code16_1.c".into(), "x".into());
        shared.insert(
            "erdgeschoss.loxl".into(),
            texts["erdgeschoss.loxl"].replace(
                r#"vx_code16_1 = Code16("Hue")"#,
                "vx_code16_1 = Code16(\"Hue\", Code: file(\"vx_code16_1.c\"))\nvx_code16_2 = Code16(\"Zwei\", Code: file(\"vx_code16_1.c\"))",
            ),
        );
        let e = Files::from_texts(&shared).unwrap_err();
        assert!(e.contains("both name file(\"vx_code16_1.c\")"), "{e}");
    }

    /// A constant takes a unit the port is documented in (#79), on a block
    /// added here and on a claim, and lox scales it into the port's unit.
    #[test]
    fn a_constant_takes_its_unit() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let mut files = Files::default();
        apply_op(
            &mut files,
            &ctx,
            &add(EG, "Monoflop", None, &[("Time", "30min")], &[]),
        )
        .unwrap();
        assert!(files.render()["erdgeschoss.loxl"].contains("Monoflop(Time: 30min)"));
        let out = compile_files(&d, &files, &mut l).unwrap();
        let uuid = l.objects["vx_monoflop_1"].uuid.clone();
        let el = find(&out.xml.root, &uuid).unwrap();
        let def = |el: &Element| {
            el.child_elements()
                .find(|c| c.name == "Co" && c.attr("K") == Some("Time"))
                .and_then(|c| c.attr("Def"))
                .map(String::from)
        };
        assert_eq!(def(el).as_deref(), Some("1800"));
        assert_eq!(crate::units::shown("Monoflop", "Time", "1800"), "30min");
        // Edited on the block: `previous` is the value as written.
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            base: &out,
            lock: &lock,
            ..ctx
        };
        let a = apply_op(&mut files, &ctx, &param(&uuid, "Time", Some("2h"))).unwrap();
        assert_eq!(a.previous, serde_json::json!("30min"));
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &param(&uuid, "Time", Some("2parsec")),
        )
        .unwrap_err();
        assert!(e.contains("unit"), "{e}");
    }

    #[test]
    fn a_new_block_is_checked_before_lox_sees_it() {
        let d = doc("house.Loxone");
        let names = BTreeSet::from(["vx_and_1".to_string()]);
        let lock = Lock::default();
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let mut files = Files::default();
        let err = |op: Op| apply_op(&mut files.clone(), &ctx, &op).unwrap_err();
        assert!(err(add(DIV, "And", None, &[], &[])).contains("no page"));
        // A virtual input goes into its folder (#84), never onto a page.
        assert!(err(add(EG, "VirtualIn", None, &[], &[])).contains("goes into the folder"));
        assert!(err(add(EG, "And", Some("a|b"), &[], &[])).contains("\"a_b\""));
        assert!(err(add(EG, "And", Some(" "), &[], &[])).contains("cannot be empty"));
        assert!(err(add(EG, "And", None, &[("Q", "1")], &[])).contains("no input Q"));
        assert!(err(add(EG, "And", None, &[("I1", "1,5")], &[])).contains("decimal point"));
        assert!(err(add(EG, "Memory", None, &[], &[("Type", "big")])).contains("\"digital\""));
        assert!(err(add(EG, "PulseAt", None, &[], &[("At", "25:00")])).contains("time of day"));
        assert!(err(add(EG, "Code16", None, &[], &[("Code", " \n")])).contains("empty"));
        assert!(err(add(EG, "Monoflop", None, &[("Time", "5%")], &[])).contains("seconds"));
        // A slug the module has is not picked again; a port by its key.
        let a = apply_op(&mut files, &ctx, &add(EG, "And", None, &[("I2", "1")], &[])).unwrap();
        assert_eq!(a.added.as_deref(), Some("vx_and_2"));
        assert!(files.render()["erdgeschoss.loxl"].contains("vx_and_2 = And(In2: 1)"));

        // Two pages with one title: lox would build on the first.
        let xml = std::fs::read_to_string(format!(
            "{}/tests/fixtures/house.Loxone",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
        .replace("Title=\"Obergeschoss\"", "Title=\"Erdgeschoss\"");
        let d = LoxoneDoc::parse(xml.as_bytes()).unwrap();
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let e = apply_op(&mut Files::default(), &ctx, &add(OG, "And", None, &[], &[])).unwrap_err();
        assert!(e.contains("2 pages are titled"), "{e}");
        apply_op(&mut Files::default(), &ctx, &add(EG, "And", None, &[], &[])).unwrap();
    }

    #[test]
    fn managed_blocks_come_from_the_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = tmp.path().join("logic.lock.json");
        std::fs::write(
            &lock,
            r#"{"objects":{"a":{"uuid":"U1","type":"And"}},"removed":{"U0":{"slug":"b","type":"Or"}}}"#,
        )
        .unwrap();
        let l = Lock::read(&lock);
        assert_eq!((l.slug_of("U1"), l.uuid_of("a")), (Some("a"), Some("U1")));
        assert!(l.placed.is_empty());
        std::fs::write(
            &lock,
            r#"{"objects":{"a":{"uuid":"U1","type":"And","page_uuid":"P","layout":{"px":96,"py":192,"px2":960,"py2":768}}}}"#,
        )
        .unwrap();
        assert_eq!(Lock::read(&lock).placed["a"].as_deref(), Some("P"));
        assert!(l.tombstoned("b"));
        assert!(Lock::read(&tmp.path().join("none")).slugs.is_empty());
    }

    /// `lock` with `moves` written in, as the next compile reads it.
    fn moved(l: &Lockfile, base: &LoxoneDoc, moves: &BTreeMap<String, Layout>) -> Lockfile {
        serde_json::from_slice(&write_moves(l.to_json().as_bytes(), base, moves).unwrap()).unwrap()
    }

    #[test]
    fn a_managed_block_is_moved_through_the_lock() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", Some("Licht"), &[], &[])).unwrap();
        // Deployed: the base draws it where lox placed it.
        let base = compile_files(&d, &files, &mut l).unwrap();
        let uuid = l.objects["vx_and_1"].uuid.clone();
        let placed = Layout::of_element(find(&base.xml.root, &uuid).unwrap()).unwrap();
        let lock = lock_of(&l);
        assert_eq!(lock.placed["vx_and_1"].as_deref(), Some(OG));
        let ctx = Ctx {
            doc: &base,
            lock: &lock,
            base: &base,
            deployed: Some(&base),
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        // Snapped to the grid, the size kept; `previous` moves it back.
        let mut files = Files::from_texts(&files.render()).unwrap();
        let (x, y) = (placed.px + 2000, placed.py + 1010);
        let a = apply_op(
            &mut files,
            &ctx,
            &Op::Move {
                uuid: uuid.clone(),
                x,
                y,
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::from_value::<Op>(a.previous).unwrap(),
            Op::Move {
                uuid: uuid.clone(),
                x: placed.px,
                y: placed.py
            }
        );
        let to = files.layout.moves["vx_and_1"];
        assert_eq!((to.px % GRID, to.py % GRID), (0, 0));
        assert_eq!((to.px, to.py), (snap(x), snap(y)));
        assert_eq!((to.w(), to.h()), (placed.w(), placed.h()));
        // Not a file: vislox's files are as they were.
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().pages,
            files.pages
        );

        // The lock alone is not enough: lox takes the base's position back
        // as a move made in Loxone Config.
        let mut plain = l.clone();
        plain.objects.get_mut("vx_and_1").unwrap().layout = Some(to);
        let out = compile_files(&base, &files, &mut plain).unwrap();
        let at = Layout::of_element(find(&out.xml.root, &uuid).unwrap()).unwrap();
        assert_eq!((at.px, at.py), (placed.px, placed.py));
        // With the position the base still shows superseded, the move holds,
        // and keeps holding on the next compile.
        let mut l2 = moved(&l, &base, &files.layout.moves);
        assert_eq!(l2.objects["vx_and_1"].superseded_layout, Some(placed));
        for _ in 0..2 {
            let out = compile_files(&base, &files, &mut l2).unwrap();
            let at = Layout::of_element(find(&out.xml.root, &uuid).unwrap()).unwrap();
            assert_eq!((at.px, at.py), (to.px, to.py));
            assert!(moves_missed(&out, &base, &lock_of(&l2), &files.layout).is_empty());
        }
        // Once applied, the base draws it there, and lox drops the line.
        let applied = compile_files(&base, &files, &mut l2).unwrap();
        compile_files(&applied, &files, &mut l2).unwrap();
        assert_eq!(l2.objects["vx_and_1"].superseded_layout, None);

        // Back where the base draws it: nothing superseded.
        let back = Placement {
            moves: BTreeMap::from([("vx_and_1".to_string(), placed)]),
            ..Default::default()
        };
        let mut l3 = moved(&l2, &base, &back.moves);
        assert_eq!(l3.objects["vx_and_1"].superseded_layout, None);
        let out = compile_files(&base, &files, &mut l3).unwrap();
        assert!(moves_missed(&out, &base, &lock_of(&l3), &back).is_empty());
        assert!(!moves_missed(&out, &base, &lock_of(&l3), &files.layout).is_empty());
    }

    #[test]
    fn only_managed_blocks_on_a_page_are_moved_or_laid_out() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", None, &[], &[])).unwrap();
        apply_op(&mut files, &ctx, &add(OG, "Or", None, &[], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let err = |op: Op| apply_op(&mut files.clone(), &ctx, &op).unwrap_err();
        // Drawn in Loxone Config: lox does not move it.
        let e = err(Op::Move {
            uuid: DIV.into(),
            x: 0,
            y: 0,
        });
        assert!(e.contains("drawn in Loxone Config"), "{e}");
        let and = l.objects["vx_and_1"].uuid.clone();
        let e = err(Op::Move {
            uuid: and.clone(),
            x: 5_000_000,
            y: 0,
        });
        assert!(e.contains("too far"), "{e}");
        assert!(
            err(Op::Move {
                uuid: "nope".into(),
                x: 0,
                y: 0
            })
            .contains("no object")
        );
        // A page without managed blocks has nothing lox lays out.
        let e = err(Op::Relayout { page: EG.into() });
        assert!(e.contains("manages no block on this page"), "{e}");
        assert!(err(Op::Relayout { page: DIV.into() }).contains("no page"));
        let a = apply_op(&mut files, &ctx, &Op::Relayout { page: OG.into() }).unwrap();
        assert_eq!(
            files.layout.relayout,
            BTreeMap::from([(OG.to_string(), "Obergeschoss".to_string())])
        );
        // `previous` puts each block back, in one `place` (#62).
        let Op::Place { moves: back } = serde_json::from_value(a.previous).unwrap() else {
            panic!("not a place")
        };
        assert_eq!(back.len(), 2);
        assert!(back.iter().any(|m| m.uuid == and));
        // A `place` moves each block as `move` does, and its previous is the
        // `place` back.
        let mut again = files.clone();
        let p = apply_op(
            &mut again,
            &ctx,
            &Op::Place {
                moves: back.clone(),
            },
        )
        .unwrap();
        assert!(
            matches!(serde_json::from_value(p.previous).unwrap(), Op::Place { moves } if moves.len() == 2)
        );
        assert_eq!(again.layout.moves.len(), 2);
        let e = apply_op(&mut again, &ctx, &Op::Place { moves: vec![] }).unwrap_err();
        assert!(e.contains("no blocks"), "{e}");
    }

    #[test]
    fn snapping_follows_loxone_configs_grid() {
        assert_eq!(snap(0), 0);
        assert_eq!(snap(47), 0);
        assert_eq!(snap(48), 96);
        assert_eq!(snap(1000), 960);
        assert_eq!(snap(-47), 0);
        assert_eq!(snap(-49), -96);
    }

    /// `doc` with the element `uuid` drawn at (`px`, `py`), its size kept.
    fn drawn_moved(doc: &LoxoneDoc, uuid: &str, px: i64, py: i64) -> LoxoneDoc {
        let at = Layout::of_element(find(&doc.xml.root, uuid).unwrap()).unwrap();
        let text = String::from_utf8(doc.to_bytes()).unwrap();
        let line = text
            .lines()
            .find(|l| l.contains(&format!("U=\"{uuid}\"")))
            .unwrap();
        let new = line
            .replace(&format!("Px=\"{}\"", at.px), &format!("Px=\"{px}\""))
            .replace(&format!("Py=\"{}\"", at.py), &format!("Py=\"{py}\""))
            .replace(
                &format!("Px2=\"{}\"", at.px2),
                &format!("Px2=\"{}\"", px + at.w()),
            )
            .replace(
                &format!("Py2=\"{}\"", at.py2),
                &format!("Py2=\"{}\"", py + at.h()),
            );
        LoxoneDoc::parse(text.replace(line, &new).as_bytes()).unwrap()
    }

    #[test]
    fn a_block_off_the_grid_is_moved_away_and_back_and_never_past_the_top_left() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", None, &[], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let uuid = l.objects["vx_and_1"].uuid.clone();
        // Deployed off the grid (moved in Loxone Config): lox takes it over.
        let placed = Layout::of_element(find(&out.xml.root, &uuid).unwrap()).unwrap();
        let (bx, by) = (placed.px + 4800 + 48, placed.py + 960 + 24);
        let base = drawn_moved(&out, &uuid, bx, by);
        let out = compile_files(&base, &files, &mut l).unwrap();
        assert_eq!(
            Layout::of_element(find(&out.xml.root, &uuid).unwrap()).map(|a| (a.px, a.py)),
            Some((bx, by))
        );
        let mv = |files: &mut Files, l: &Lockfile, out: &LoxoneDoc, x, y| {
            let lock = lock_of(l);
            let ctx = Ctx {
                doc: out,
                lock: &lock,
                base: &base,
                deployed: Some(&base),
                names: &names,
                lets: &Default::default(),
                drawn: &Default::default(),
                adoptions: &Default::default(),
                page_adoptions: &Default::default(),
                claimed: &Default::default(),
                templates: &Default::default(),
                module: &Default::default(),
            };
            files.layout = Placement::default();
            apply_op(
                files,
                &ctx,
                &Op::Move {
                    uuid: uuid.clone(),
                    x,
                    y,
                },
            )
            .unwrap()
        };
        // Away: snapped to the grid.
        let a = mv(&mut files, &l, &out, bx + 2000, by);
        assert_eq!(
            serde_json::from_value::<Op>(a.previous).unwrap(),
            Op::Move {
                uuid: uuid.clone(),
                x: bx,
                y: by
            }
        );
        let to = files.layout.moves["vx_and_1"];
        assert_eq!((to.px, to.py), (snap(bx + 2000), snap(by)));
        let mut l2 = moved(&l, &base, &files.layout.moves);
        let away = compile_files(&base, &files, &mut l2).unwrap();
        assert!(moves_missed(&away, &base, &lock_of(&l2), &files.layout).is_empty());

        // Back (Revert): to the deployed position as it is, off the grid; lox
        // keeps the base's rectangle, and that counts as kept.
        mv(&mut files, &l2, &away, bx, by);
        let to = files.layout.moves["vx_and_1"];
        assert_eq!((to.px, to.py), (bx, by));
        let mut l3 = moved(&l2, &base, &files.layout.moves);
        assert_eq!(l3.objects["vx_and_1"].superseded_layout, None);
        let back = compile_files(&base, &files, &mut l3).unwrap();
        assert!(moves_missed(&back, &base, &lock_of(&l3), &files.layout).is_empty());
        // A snapped target within a grid step of it is kept the same way.
        let near = Placement {
            moves: BTreeMap::from([(
                "vx_and_1".to_string(),
                Layout::at(snap(bx), snap(by), to.w(), to.h()),
            )]),
            ..Default::default()
        };
        assert!(moves_missed(&back, &base, &lock_of(&l3), &near).is_empty());

        // Never past the page's top left.
        mv(&mut files, &l3, &back, -500, -10);
        let to = files.layout.moves["vx_and_1"];
        assert_eq!((to.px, to.py), (0, 0));
    }

    #[test]
    fn only_moves_on_a_page_laid_out_afresh_go_unchecked() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", None, &[], &[])).unwrap();
        apply_op(&mut files, &ctx, &add(EG, "Or", None, &[], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        // Neither move is in `out`: only the one on the page not laid out is missed.
        let far = Layout::at(19200, 9600, 1344, 696);
        let place = Placement {
            moves: BTreeMap::from([("vx_and_1".to_string(), far), ("vx_or_1".to_string(), far)]),
            relayout: BTreeMap::from([(OG.to_string(), "Obergeschoss".to_string())]),
        };
        assert_eq!(moves_missed(&out, &d, &lock, &place), ["vx_or_1"]);
        let none = Placement {
            relayout: BTreeMap::new(),
            ..place
        };
        assert_eq!(
            moves_missed(&out, &d, &lock, &none),
            ["vx_and_1", "vx_or_1"]
        );
    }

    #[test]
    fn a_lock_entry_without_a_uuid_is_refused() {
        let d = doc("house.Loxone");
        let moves = BTreeMap::from([("a".to_string(), Layout::at(0, 0, 96, 96))]);
        for lock in [
            r#"{"objects":{"a":{"type":"And"}}}"#,
            r#"{"objects":{"a":{"uuid":"","type":"And"}}}"#,
        ] {
            let e = write_moves(lock.as_bytes(), &d, &moves).unwrap_err();
            assert!(e.contains("no UUID for block a"), "{e}");
        }
        let e = write_moves(br#"{"objects":{}}"#, &d, &moves).unwrap_err();
        assert!(e.contains("no block a"), "{e}");
    }

    #[test]
    fn arrange_says_why_a_page_cannot_be_laid_out() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", None, &[], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let mut lock = lock_of(&l);
        assert_eq!(
            relayout_title(&out, &out, &lock, OG).unwrap(),
            "Obergeschoss"
        );
        assert!(
            relayout_title(&out, &out, &lock, EG)
                .unwrap_err()
                .contains("manages no block")
        );
        // A lock from before lox pinned pages.
        lock.placed.insert("vx_and_1".into(), None);
        assert!(
            relayout_title(&out, &out, &lock, OG)
                .unwrap_err()
                .contains("older lox")
        );
        // Another page with its title.
        let twice = LoxoneDoc::parse(
            String::from_utf8(out.to_bytes())
                .unwrap()
                .replace("Title=\"Erdgeschoss\"", "Title=\"Obergeschoss\"")
                .as_bytes(),
        )
        .unwrap();
        let lock = lock_of(&l);
        assert!(
            relayout_title(&twice, &twice, &lock, OG)
                .unwrap_err()
                .contains("2 pages are titled")
        );
    }

    // ── wires (#61) ─────────────────────────────────────────────────────

    const OR: &str = "10000009-0000-0090-ffff504f94000001";

    fn ends(f: (&str, &str), t: (&str, &str)) -> ([String; 2], [String; 2]) {
        (
            [f.0.to_string(), f.1.to_string()],
            [t.0.to_string(), t.1.to_string()],
        )
    }

    fn wire_op(f: (&str, &str), t: (&str, &str)) -> Op {
        let (from, to) = ends(f, t);
        Op::Wire { from, to }
    }

    fn unwire_op(f: (&str, &str), t: (&str, &str)) -> Op {
        let (from, to) = ends(f, t);
        Op::Unwire { from, to }
    }

    /// Whether `doc` wires the connector `from` of one object into `to` of another.
    fn has_wire(doc: &LoxoneDoc, f: (&str, &str), t: (&str, &str)) -> bool {
        let co = |u: &str, k: &str| {
            find(&doc.xml.root, u)
                .unwrap()
                .child_elements()
                .find(|c| c.name == "Co" && c.attr("K") == Some(k))
                .unwrap()
        };
        let src = co(f.0, f.1).attr("U").unwrap().to_string();
        co(t.0, t.1)
            .child_elements()
            .any(|c| c.name == "In" && c.attr("Input") == Some(src.as_str()))
    }

    #[test]
    fn a_wire_between_objects_loxone_config_drew_is_compiled_by_lox_and_taken_out_again() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        // The Divide block's output onto the upper floor's OR: another page.
        let w = wire_op((DIV, "AQ"), (OR, "I2"));
        let a = apply_op(&mut files, &ctx, &w).unwrap();
        assert_eq!(
            a.previous,
            serde_json::to_value(unwire_op((DIV, "AQ"), (OR, "I2"))).unwrap()
        );
        let text = &files.render()["edits.loxl"];
        let (sd, so) = (slug(DIV), slug(OR));
        assert!(
            text.contains(&format!("extern {sd} = Div(uuid: \"{DIV}\")\n")),
            "{text}"
        );
        assert!(text.contains(&format!("{so}.In2 <- {sd}.AQ\n")), "{text}");
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().claims,
            files.claims
        );
        assert_eq!(
            wire_ends(&files, &lock),
            vec![ends((DIV, "AQ"), (OR, "I2"))]
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert!(has_wire(&out, (DIV, "AQ"), (OR, "I2")));
        // The wire Loxone Config drew into the other input stays.
        assert!(has_wire(
            &out,
            ("10000004-0000-0040-ffff504f94000001", "Q"),
            (OR, "I1")
        ));
        assert_eq!(l.extern_wires.len(), 1);

        // Taken out again: the statement and the externs go, and lox takes the wire out.
        let lock = lock_of(&l);
        let drawn = files.claims.1.clone();
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &drawn,
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        // One request may take a wire out and draw it again (an undo and a
        // redo, a reconnect to the same port): the ops see each other.
        let u = unwire_op((DIV, "AQ"), (OR, "I2"));
        let mut again = files.clone();
        apply_op(&mut again, &ctx, &u).unwrap();
        let e = apply_op(&mut again.clone(), &ctx, &u).unwrap_err();
        assert!(e.contains("is not wired"), "{e}");
        apply_op(&mut again, &ctx, &w).unwrap();
        assert_eq!(again, files);
        let e = apply_op(&mut again, &ctx, &w).unwrap_err();
        assert!(e.contains("already wired"), "{e}");
        assert_eq!(
            apply_op(&mut files, &ctx, &u).unwrap().previous,
            serde_json::to_value(&w).unwrap()
        );
        assert_eq!(files.claims, Claims::default());
        let again = compile_files(&d, &files, &mut l).unwrap();
        assert!(!has_wire(&again, (DIV, "AQ"), (OR, "I2")));
        assert!(l.extern_wires.is_empty());
    }

    #[test]
    fn a_wire_is_refused_where_loxone_config_or_lox_would_not_have_it() {
        const AND_Q: (&str, &str) = (AND, "Q");
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let lock = Lock::default();
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let err = |op: Op| apply_op(&mut Files::default(), &ctx, &op).unwrap_err();
        assert!(err(wire_op((DIV, "Input2"), (OR, "I2"))).contains("is an input"));
        assert!(err(wire_op((DIV, "AQ"), (AND, "Q"))).contains("is an output"));
        assert!(err(wire_op((DIV, "AQ"), (DIV, "Input2"))).contains("another block"));
        assert!(err(wire_op((DIV, "AQ"), (OR, "Nope"))).contains("no connector"));
        // Loxone Config drew this one already.
        assert!(err(wire_op(AND_Q, (OR, "I1"))).contains("already wired"));
        // ...and only Loxone Config can take it out.
        let e = err(unwire_op(AND_Q, (OR, "I1")));
        assert!(e.contains("Drawn in Loxone Config"), "{e}");
        assert!(err(unwire_op((DIV, "AQ"), (OR, "I2"))).contains("not wired"));

        // A text output goes only into a text input.
        let xml = std::fs::read_to_string(format!(
            "{}/tests/fixtures/house.Loxone",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
        .replace(
            r#"Analog="true" LinkRefType="71""#,
            r#"Text="true" LinkRefType="71""#,
        );
        let t = LoxoneDoc::parse(xml.as_bytes()).unwrap();
        let ref_el = find(&t.xml.root, REF).unwrap();
        assert_eq!(signal("InputRef", Some(ref_el), "AQ"), Signal::Text);
        let el = |u: &str| find(&d.xml.root, u);
        assert_eq!(signal("InputRef", el(REF), "AQ"), Signal::Analog);
        assert_eq!(signal("And", el(AND), "Q"), Signal::Digital);
        // From lox's tables: a virtual input's value, a text output, a
        // memory flag's type; a connector lox does not list is unknown.
        assert_eq!(signal("VirtualIn", el(VI), "Q"), Signal::Digital);
        assert_eq!(signal("VirtualTextIn", None, "Q"), Signal::Text);
        assert_eq!(signal("Status", None, "TQ"), Signal::Text);
        let mem =
            loxone_config::xml::XmlDocument::parse(br#"<C Type="Memory" Tp="2" U="x"/>"#).unwrap();
        assert_eq!(signal("Memory", Some(&mem.root), "Q"), Signal::Text);
        assert_eq!(signal("Banana", None, "Q"), Signal::Unknown);
        assert_eq!(signal("And", None, "Nope"), Signal::Unknown);
        let tctx = Ctx {
            doc: &t,
            lock: &lock,
            base: &t,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let e = apply_op(
            &mut Files::default(),
            &tctx,
            &wire_op((REF, "AQ"), (OR, "I2")),
        )
        .unwrap_err();
        assert!(e.contains("text only to text"), "{e}");
        // Analog into digital is fine: a value other than 0 is on.
        apply_op(
            &mut Files::default(),
            &ctx,
            &wire_op((REF, "AQ"), (OR, "I2")),
        )
        .unwrap();

        // A block the rest of the module manages is wired in its own source,
        // and the wires it draws are taken out there.
        let managed = Lock {
            slugs: BTreeMap::from([("teiler".to_string(), DIV.to_string())]),
            wires: BTreeSet::from([(
                "10000003-0000-0032-01ff504f94000001".to_string(),
                "10000006-0000-0061-00ff504f94000001".to_string(),
            )]),
            ..Lock::default()
        };
        let mctx = Ctx {
            doc: &d,
            lock: &managed,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let e = apply_op(
            &mut Files::default(),
            &mctx,
            &wire_op((DIV, "AQ"), (OR, "I2")),
        )
        .unwrap_err();
        assert!(e.contains("module manages"), "{e}");
        let e = apply_op(
            &mut Files::default(),
            &mctx,
            &unwire_op((REF, "AQ"), (DIV, "Input1")),
        )
        .unwrap_err();
        assert!(e.contains("module draws this wire"), "{e}");
        // An input with a constant edited here is cleared first.
        let mut files = Files::default();
        apply_op(&mut files, &ctx, &param(OR, "I2", Some("1"))).unwrap();
        let e = apply_op(&mut files, &ctx, &wire_op((DIV, "AQ"), (OR, "I2"))).unwrap_err();
        assert!(e.contains("clear it first"), "{e}");
    }

    #[test]
    fn a_block_added_here_is_wired_both_ways_and_its_wires_go_with_it() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", None, &[("I2", "1")], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let new = l.objects["vx_and_1"].uuid.clone();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        // Its input with a constant takes no wire; the other does.
        let e = apply_op(&mut files, &ctx, &wire_op((DIV, "AQ"), (&new, "I2"))).unwrap_err();
        assert!(e.contains("clear it first"), "{e}");
        apply_op(&mut files, &ctx, &wire_op((DIV, "AQ"), (&new, "I1"))).unwrap();
        apply_op(&mut files, &ctx, &wire_op((&new, "Q"), (OR, "I2"))).unwrap();
        let text = &files.render()["edits.loxl"];
        assert!(text.contains("vx_and_1.In1 <- "), "{text}");
        assert!(
            text.contains(&format!("{}.In2 <- vx_and_1.Q\n", slug(OR))),
            "{text}"
        );
        let mut ends_now = wire_ends(&files, &lock);
        ends_now.sort();
        let mut want = vec![
            ends((DIV, "AQ"), (&new, "I1")),
            ends((&new, "Q"), (OR, "I2")),
        ];
        want.sort();
        assert_eq!(ends_now, want);
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert!(has_wire(&out, (DIV, "AQ"), (&new, "I1")));
        assert!(has_wire(&out, (&new, "Q"), (OR, "I2")));

        // Taken out, the block takes its wires along, and the externs they needed.
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let undo = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: new.clone() })
            .unwrap()
            .previous;
        assert!(
            files.claims.1.is_empty() && files.claims.0.is_empty(),
            "{:?}",
            files.claims
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert!(find(&out.xml.root, &new).is_none());
        assert!(!has_wire(&out, (DIV, "AQ"), (OR, "I2")));

        // Its `previous` adds it again with its wires: a complete undo. lox
        // mints new UUIDs, so the block names itself by an empty UUID.
        let undo: Op = serde_json::from_value(undo).unwrap();
        let Op::AddBlock { wires, .. } = &undo else {
            panic!("{undo:?}")
        };
        let mut got: Vec<_> = wires
            .iter()
            .map(|w| (w.from.clone(), w.to.clone()))
            .collect();
        got.sort();
        let mut want = vec![ends((DIV, "AQ"), ("", "I1")), ends(("", "Q"), (OR, "I2"))];
        want.sort();
        assert_eq!(got, want);
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let a = apply_op(&mut files, &ctx, &undo).unwrap();
        let slug2 = a.added.unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let back = l.objects[&slug2].uuid.clone();
        assert_ne!(back, new);
        assert!(has_wire(&out, (DIV, "AQ"), (&back, "I1")));
        assert!(has_wire(&out, (&back, "Q"), (OR, "I2")));
        assert_eq!(
            find(&out.xml.root, &back).unwrap().attr("Type"),
            Some("And")
        );
    }

    #[test]
    fn a_wire_whose_end_the_files_do_not_declare_is_refused() {
        let s = slug(DIV);
        let page = "# page uuid: 10000008-0000-0080-ffff504f94000001\npage \"Obergeschoss\"\n\nvx_and_1 = And()\n";
        let files = |claims: String| {
            BTreeMap::from([
                ("edits.loxl".to_string(), claims),
                ("obergeschoss.loxl".to_string(), page.to_string()),
            ])
        };
        let ok = format!("extern {s} = Div(uuid: \"{DIV}\")\nvx_and_1.In1 <- {s}.AQ\n");
        assert_eq!(Files::from_texts(&files(ok)).unwrap().claims.1.len(), 1);
        // Its extern deleted by hand: refused, not written back to break the compile.
        let e = Files::from_texts(&files(format!("vx_and_1.In1 <- {s}.AQ\n"))).unwrap_err();
        assert!(e.contains(&format!("names `{s}`")), "{e}");
        // A block no file of vislox's declares.
        let e = Files::from_texts(&files(format!(
            "extern {s} = Div(uuid: \"{DIV}\")\nvx_or_9.In1 <- {s}.AQ\n"
        )))
        .unwrap_err();
        assert!(e.contains("names `vx_or_9`"), "{e}");
    }

    #[test]
    fn wires_in_the_file_are_read_back_and_foreign_ones_refused() {
        let u = DIV;
        let s = slug(u);
        let text =
            format!("extern {s} = Div(uuid: \"{u}\", Input2: vx_and_1.Q)\nvx_or_1.In1 <- {s}.AQ\n");
        let c = Claims::parse(&text).unwrap();
        assert_eq!(c.1.len(), 2);
        assert!(c.0[u].is_empty());
        // Rendered with the wires as statements, and read back the same.
        assert_eq!(Claims::parse(&c.render()).unwrap(), c);
        let e = Claims::parse(&format!(
            "extern {s} = Div(uuid: \"{u}\")\n{s}.In2 <- licht.Q\n"
        ))
        .unwrap_err();
        assert!(e.contains("did not write"), "{e}");
    }

    /// Handing back one of two blocks adopted together (#72): the other keeps
    /// every wire between them, onto vislox's extern of the one handed back
    /// (a loop's wire, a binding in either declaration).
    #[test]
    fn a_block_handed_back_leaves_its_neighbours_wires_on_an_extern() {
        const METER: &str = "10000005-0000-0050-ffff504f94000001";
        let d = doc("house.Loxone");
        let lock = Lock {
            slugs: BTreeMap::from([
                ("vx_a".to_string(), AND.to_string()),
                ("vx_b".to_string(), METER.to_string()),
            ]),
            ..Default::default()
        };
        let text = |a: &str, b: &str, wire: &str| {
            format!(
                "page \"Erdgeschoss\"\n# Adopted from Loxone Config\nvx_a = And(\"O1001\"{a})\nvx_b = MeterAbsUni(\"Zähler\"{b})\n{wire}"
            )
        };
        let files = |page: String| {
            let mut f =
                Files::from_texts(&BTreeMap::from([("eg.loxl".to_string(), page)])).unwrap();
            f.repin();
            f
        };
        let (adoptions, pages) = (Default::default(), Default::default());
        let ctx = Ctx {
            doc: &d,
            base: &d,
            lock: &lock,
            deployed: None,
            names: &Default::default(),
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &adoptions,
            page_adoptions: &pages,
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let back = |f: &mut Files, u: &str| {
            apply_op(
                f,
                &ctx,
                &Op::Unadopt {
                    uuids: vec![u.into()],
                },
            )
            .unwrap();
            f.render()
        };
        let ext_a = slug(AND);
        // A loop's wire, out of the block that stays into the one handed back.
        let mut f = files(text("", "", "vx_a.In2 <- vx_b\n"));
        let out = back(&mut f, AND);
        assert!(
            out["eg.loxl"].contains(&format!("{ext_a}.In2 <- vx_b\n")),
            "{out:?}"
        );
        assert!(!out["eg.loxl"].contains("vx_a "), "{out:?}");
        assert!(
            out[CLAIMS].contains(&format!("extern {ext_a} = And(uuid: \"{AND}\")")),
            "{out:?}"
        );
        assert!(f.lock_edits.drop.contains("vx_a"));
        // A binding in the other block's declaration names its extern.
        let mut f = files(text("", ", Power: vx_a", ""));
        let out = back(&mut f, AND);
        assert!(
            out["eg.loxl"].contains(&format!("Power: {ext_a})")),
            "{out:?}"
        );
        // A binding in its own declaration, fed by the block that stays: that
        // block's wire onto its extern now.
        let mut f = files(text("", ", Power: vx_a", ""));
        let out = back(&mut f, METER);
        let ext_b = slug(METER);
        assert!(
            out["eg.loxl"].contains(&format!("{ext_b}.Power <- vx_a\n")),
            "{out:?}"
        );
        assert!(!out["eg.loxl"].contains("vx_b"), "{out:?}");
        // Everything the files say still reads back.
        Files::from_texts(&out).unwrap();
    }

    /// An interval in a test: from, to, value.
    type Sp = (u32, u32, Option<&'static str>);

    const UHR: &str = "10000020-0000-0200-ffff504f94000001";
    const HEIZ: &str = "10000021-0000-0210-ffff504f94000001";
    const LICHT: &str = "10000022-0000-0220-ffff504f94000001";
    const PARTY: &str = "10000011-0000-0110-ffff504f94000001";

    /// The fixture with three Daytimers on the upper floor: Uhr and Heizung
    /// (analog; Uhr with a Monday and a Party column), Licht (digital).
    fn timers() -> LoxoneDoc {
        let p = format!("{}/tests/fixtures/house.Loxone", env!("CARGO_MANIFEST_DIR"));
        let mut s = std::fs::read_to_string(p).unwrap();
        let timer = |u: &str, n: u32, title: &str, analog: bool, modes: &str, body: &str| {
            format!(
                "\t\t\t<C Type=\"DayTimer\" V=\"178\" U=\"{u}\" Title=\"{title}\" Px=\"960\" Py=\"{}\" \
                 Px2=\"3648\" Py2=\"{}\" Nio=\"2\" WF=\"16384\"{}{}>\
                 <Co K=\"Mode\" U=\"{}\"/><Co K=\"AQ\" U=\"{}\"/>{body}</C>\n",
                4800 + 960 * n,
                5600 + 960 * n,
                if analog {
                    " Analog=\"true\" DefValue=\"18\""
                } else {
                    ""
                },
                if modes.is_empty() {
                    String::new()
                } else {
                    format!(" Modes=\"{modes}\"")
                },
                u.replacen("0-ffff", "1-00ff", 1),
                u.replacen("0-ffff", "2-01ff", 1),
            )
        };
        let all = timer(
            UHR,
            0,
            "Uhr",
            true,
            &format!("00000000-0000-0004-1500000000000000,{PARTY}"),
            "<Entry From=\"390\" To=\"480\" V=\"21\"/><Entry From=\"1080\" To=\"1380\" Ix=\"1\" V=\"22.5\"/>",
        ) + &timer(HEIZ, 1, "Heizung", true, "", "")
            + &timer(LICHT, 2, "Licht", false, "", "");
        let end = "\n\t\t</C>\n\t</C>\n</ControlList>";
        let at = s.rfind(end).expect("end of the upper floor");
        s.insert_str(at + 1, &all);
        LoxoneDoc::parse(s.as_bytes()).unwrap()
    }

    fn sched(set: Option<&str>, cols: &[(&str, &[Sp])]) -> Option<crate::sched_edit::Sched> {
        Some(crate::sched_edit::Sched {
            set: set.map(Into::into),
            modes: (!cols.is_empty()).then(|| {
                cols.iter()
                    .map(|(u, spans)| {
                        let v = spans
                            .iter()
                            .map(|(from, to, value)| crate::sched_edit::Span {
                                from: *from,
                                to: *to,
                                value: value.map(Into::into),
                                activate: false,
                            })
                            .collect();
                        (u.to_string(), v)
                    })
                    .collect()
            }),
            ..Default::default()
        })
    }

    fn entries(doc: &LoxoneDoc, uuid: &str) -> Vec<(u32, u32, f64)> {
        let t = crate::schedule::read(find(&doc.xml.root, uuid).unwrap(), uuid, |_| None).unwrap();
        let mut v: Vec<_> = t.entries.iter().map(|e| (e.from, e.to, e.value)).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    }

    #[test]
    fn switching_times_are_a_schedule_set_lox_writes_onto_the_timer() {
        use crate::sched_edit::builtin_uuid;
        let d = timers();
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let claimed = BTreeMap::new();
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &claimed,
            templates: &Default::default(),
            module: &Default::default(),
        };
        let (mon, sat, hol) = (builtin_uuid(4), builtin_uuid(9), builtin_uuid(1));
        let wk: Vec<(String, Vec<Sp>)> = (4..=8)
            .map(|n| {
                (
                    builtin_uuid(n),
                    vec![(360, 480, Some("21.5")), (1020, 1350, Some("21.5"))],
                )
            })
            .collect();
        let mut cols: Vec<(&str, &[Sp])> =
            wk.iter().map(|(u, v)| (u.as_str(), v.as_slice())).collect();
        let we = [(480, 1380, Some("20"))];
        cols.push((&sat, &we));
        let sun = builtin_uuid(10);
        cols.push((&sun, &we));
        cols.push((&hol, &[]));
        let party = [(1080, 1440, Some("23"))];
        cols.push((PARTY, &party));
        let op = Op::Schedule {
            uuid: UHR.into(),
            value: sched(None, &cols),
        };
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        assert_eq!(a.previous, serde_json::Value::Null);
        let text = &files.render()["edits.loxl"];
        let s = slug(UHR);
        for want in [
            format!("extern {s} = DayTimer(uuid: \"{UHR}\")\n"),
            "schedule vx_uhr_schedule\n\tweekdays 06:00-08:00 = 21.5, 17:00-22:30 = 21.5\n\
             \tweekend 08:00-23:00 = 20\n\tholiday off\n\t\"Party\" 18:00-24:00 = 23\nend\n"
                .to_string(),
            format!("{s}.schedule = vx_uhr_schedule\n"),
        ] {
            assert!(text.contains(&want), "{want} in\n{text}");
        }
        // Read back as written.
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().claims,
            files.claims
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        let e = entries(&out, UHR);
        assert!(
            e.contains(&(360, 480, 21.5)) && e.contains(&(1080, 1440, 23.0)),
            "{e:?}"
        );
        assert_eq!(e.len(), 5 * 2 + 2 + 1);
        assert!(crate::sched_edit::missed(&files, &out, &lock, &[UHR.into()], &[]).is_empty());
        assert!(crate::sched_edit::differs(&out, &d, UHR));

        // Changed again: its own set is rewritten, and the previous is what it claimed.
        let ctx2 = Ctx { doc: &out, ..ctx };
        let op2 = Op::Schedule {
            uuid: UHR.into(),
            value: sched(None, &[(&mon, &[(0, 60, Some("5"))])]),
        };
        let a2 = apply_op(&mut files, &ctx2, &op2).unwrap();
        let prev: Option<crate::sched_edit::Sched> =
            serde_json::from_value(a2.previous.clone()).unwrap();
        let prev = prev.unwrap();
        assert_eq!(prev.set.as_deref(), Some("vx_uhr_schedule"));
        assert_eq!(prev.modes.as_ref().unwrap()[&mon].len(), 2);
        assert_eq!(prev.modes.as_ref().unwrap()[PARTY][0].to, 1440);
        assert_eq!(files.claims.2.sets.len(), 1);
        let undo = undo_ops(std::slice::from_ref(&op2), &[a2.previous], &[]).unwrap();
        assert_eq!(
            undo,
            [Op::Schedule {
                uuid: UHR.into(),
                value: Some(prev)
            }]
        );

        // Heizung shares Uhr's set (Use the same times as …); Licht is digital and may not.
        let share = Op::Schedule {
            uuid: HEIZ.into(),
            value: sched(Some("vx_uhr_schedule"), &[]),
        };
        apply_op(&mut files, &ctx2, &share).unwrap();
        let e = apply_op(
            &mut files.clone(),
            &ctx2,
            &Op::Schedule {
                uuid: LICHT.into(),
                value: sched(Some("vx_uhr_schedule"), &[]),
            },
        )
        .unwrap_err();
        assert!(e.contains("Licht is digital"), "{e}");
        // Heizung on its own: a new set, Uhr's stays.
        apply_op(
            &mut files,
            &ctx2,
            &Op::Schedule {
                uuid: HEIZ.into(),
                value: sched(None, &[(&mon, &[(60, 120, Some("19"))])]),
            },
        )
        .unwrap();
        assert_eq!(files.claims.2.sets.len(), 2);
        assert_eq!(files.claims.2.claims[&slug(HEIZ)], "vx_heizung_schedule");
        // Both on Uhr's set, rewritten for both.
        apply_op(&mut files, &ctx2, &share).unwrap();
        assert_eq!(
            files.claims.2.sets.len(),
            1,
            "Heizung's own set went with its last claim"
        );
        apply_op(
            &mut files,
            &ctx2,
            &Op::Schedule {
                uuid: HEIZ.into(),
                value: sched(Some("vx_uhr_schedule"), &[(&sat, &[(0, 1440, Some("16"))])]),
            },
        )
        .unwrap();
        let mut l2 = l.clone();
        let out2 = compile_files(&d, &files, &mut l2).unwrap();
        assert_eq!(entries(&out2, UHR), entries(&out2, HEIZ));
        assert_eq!(entries(&out2, UHR), [(0, 1440, 16.0)]);
        assert!(crate::sched_edit::missed(&files, &out2, &lock, &[HEIZ.into()], &[]).is_empty());

        // A digital timer: no values.
        let e = apply_op(
            &mut files.clone(),
            &ctx2,
            &Op::Schedule {
                uuid: LICHT.into(),
                value: sched(None, &[(&mon, &[(60, 120, Some("3"))])]),
            },
        )
        .unwrap_err();
        assert!(e.contains("digital"), "{e}");
        apply_op(
            &mut files,
            &ctx2,
            &Op::Schedule {
                uuid: LICHT.into(),
                value: sched(None, &[(&mon, &[(60, 120, None)])]),
            },
        )
        .unwrap();
        assert!(files.render()["edits.loxl"].contains("\tmon 01:00-02:00\n"));

        // Dropped: the timer is as the base has it, and the set and extern go.
        for u in [UHR, HEIZ, LICHT] {
            apply_op(
                &mut files,
                &ctx2,
                &Op::Schedule {
                    uuid: u.into(),
                    value: None,
                },
            )
            .unwrap();
        }
        assert!(files.claims.2.is_empty() && files.claims.0.is_empty());
        let out3 = compile_files(&d, &files, &mut l2).unwrap();
        assert_eq!(entries(&out3, UHR), entries(&d, UHR));

        // Not a DayTimer, a timer the module claims elsewhere, a mode the config lacks.
        assert!(
            apply_op(
                &mut files,
                &ctx,
                &Op::Schedule {
                    uuid: DIV.into(),
                    value: sched(None, &[(&mon, &[])])
                }
            )
            .unwrap_err()
            .contains("only a DayTimer")
        );
        let elsewhere = BTreeMap::from([(
            "schedule",
            BTreeMap::from([(UHR.to_string(), "pages/uhr.loxl:4".to_string())]),
        )]);
        let e = apply_op(
            &mut files,
            &Ctx {
                claimed: &elsewhere,
                ..ctx
            },
            &op,
        )
        .unwrap_err();
        assert!(e.contains("pages/uhr.loxl:4 claims"), "{e}");
        let e = apply_op(
            &mut files,
            &ctx,
            &Op::Schedule {
                uuid: UHR.into(),
                value: sched(None, &[("00000000-0000-0000-0000-000000000000", &[])]),
            },
        )
        .unwrap_err();
        assert!(e.contains("no operating mode"), "{e}");
    }

    /// Rewriting a set other timers share is undone whole: the undo puts
    /// their switching times back too, not only this timer's claim (#73).
    #[test]
    fn undoing_a_rewrite_of_a_shared_set_gives_the_others_their_times_back() {
        use crate::sched_edit::builtin_uuid;
        let d = timers();
        let names = BTreeSet::new();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let mon = builtin_uuid(4);
        let mut files = Files::default();
        // Uhr on its set; Heizung on one of its own.
        apply_op(
            &mut files,
            &ctx,
            &Op::Schedule {
                uuid: UHR.into(),
                value: sched(None, &[(&mon, &[(60, 120, Some("20"))])]),
            },
        )
        .unwrap();
        apply_op(
            &mut files,
            &ctx,
            &Op::Schedule {
                uuid: HEIZ.into(),
                value: sched(None, &[(&mon, &[(0, 30, Some("17"))])]),
            },
        )
        .unwrap();
        let start = files.clone();
        let uhr_before = entries(&compile_files(&d, &files, &mut l.clone()).unwrap(), UHR);
        for (label, heiz) in [("its own set", true), ("no claim", false)] {
            files = start.clone();
            if !heiz {
                apply_op(
                    &mut files,
                    &ctx,
                    &Op::Schedule {
                        uuid: HEIZ.into(),
                        value: None,
                    },
                )
                .unwrap();
            }
            let was = files.clone();
            // Heizung rewrites Uhr's set: both get 05:00-06:00.
            let op = Op::Schedule {
                uuid: HEIZ.into(),
                value: sched(
                    Some("vx_uhr_schedule"),
                    &[(&mon, &[(300, 360, Some("21"))])],
                ),
            };
            let a = apply_op(&mut files, &ctx, &op).unwrap();
            let out = compile_files(&d, &files, &mut l).unwrap();
            assert_eq!(entries(&out, UHR), [(300, 360, 21.0)], "{label}");
            let undo = undo_ops(
                std::slice::from_ref(&op),
                std::slice::from_ref(&a.previous),
                &[],
            )
            .unwrap();
            let Op::Schedule { value: Some(v), .. } = &undo[0] else {
                panic!("{label}: {undo:?}")
            };
            assert_eq!(
                v.restore[&"vx_uhr_schedule".to_string()][&mon][0].from,
                60,
                "{label}"
            );
            assert_eq!(v.set.is_some(), heiz, "{label}");
            // Undone: Uhr has its times again, Heizung its claim as it was.
            let b = apply_op(&mut files, &ctx, &undo[0]).unwrap();
            assert_eq!(files.claims, was.claims, "{label}");
            let out = compile_files(&d, &files, &mut l).unwrap();
            assert_eq!(entries(&out, UHR), uhr_before, "{label}");
            assert!(
                crate::sched_edit::missed(
                    &files,
                    &out,
                    &lock,
                    &[HEIZ.into()],
                    &["vx_uhr_schedule".into()]
                )
                .is_empty()
            );
            // And the undo's own previous redoes it whole.
            let redo = undo_ops(std::slice::from_ref(&undo[0]), &[b.previous], &[]).unwrap();
            apply_op(&mut files, &ctx, &redo[0]).unwrap();
            let out = compile_files(&d, &files, &mut l).unwrap();
            assert_eq!(entries(&out, UHR), [(300, 360, 21.0)], "{label}");
            assert_eq!(entries(&out, HEIZ), [(300, 360, 21.0)], "{label}");
        }
    }

    /// A timer the module's own file claims too: vislox's claim can still be
    /// dropped (every compile fails while both claim it), not changed (#73).
    #[test]
    fn a_claim_on_a_timer_the_module_claims_too_can_be_dropped() {
        use crate::sched_edit::builtin_uuid;
        let d = timers();
        let names = BTreeSet::new();
        let lock = Lock::default();
        let mut ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            lets: &Default::default(),
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            claimed: &Default::default(),
            templates: &Default::default(),
            module: &Default::default(),
        };
        let mut files = Files::default();
        let mon = builtin_uuid(4);
        apply_op(
            &mut files,
            &ctx,
            &Op::Schedule {
                uuid: UHR.into(),
                value: sched(None, &[(&mon, &[(60, 120, Some("20"))])]),
            },
        )
        .unwrap();
        let elsewhere = BTreeMap::from([(
            "schedule",
            BTreeMap::from([(UHR.to_string(), "pages/uhr.loxl:4".to_string())]),
        )]);
        ctx.claimed = &elsewhere;
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &Op::Schedule {
                uuid: UHR.into(),
                value: sched(None, &[(&mon, &[])]),
            },
        )
        .unwrap_err();
        assert!(e.contains("pages/uhr.loxl:4 claims"), "{e}");
        apply_op(
            &mut files,
            &ctx,
            &Op::Schedule {
                uuid: UHR.into(),
                value: None,
            },
        )
        .unwrap();
        assert!(files.claims.2.is_empty() && files.claims.0.is_empty());
    }

    #[test]
    fn a_schedule_claim_vislox_did_not_write_is_refused() {
        let s = slug(UHR);
        let ext = format!("extern {s} = DayTimer(uuid: \"{UHR}\")\n");
        let set = "schedule vx_a\n\tmon 01:00-02:00\nend\n";
        let read = |t: String| Files::from_texts(&BTreeMap::from([("edits.loxl".to_string(), t)]));
        assert!(read(format!("{ext}{set}{s}.schedule = vx_a\n")).is_ok());
        // A set of the module's own name, a claim without its set or its extern.
        assert!(
            read(format!(
                "{ext}schedule abend\n\tmon 01:00-02:00\nend\n{s}.schedule = abend\n"
            ))
            .is_err()
        );
        assert!(read(format!("{ext}{s}.schedule = vx_b\n")).is_err());
        assert!(
            read(format!("{set}{s}.schedule = vx_a\n"))
                .unwrap_err()
                .contains("neither an extern")
        );
    }

    // ── lighting moods (#74) ───────────────────────────────────────────

    fn mrow(name: &str, values: &str) -> crate::mood_edit::MoodRow {
        crate::mood_edit::MoodRow {
            name: name.into(),
            values: values.into(),
        }
    }

    fn moods_op(uuid: &str, set: Option<&str>, rows: &[(&str, &str)]) -> Op {
        Op::Moods {
            uuid: uuid.into(),
            value: Some(crate::mood_edit::MoodsVal {
                set: set.map(Into::into),
                moods: (!rows.is_empty()).then(|| rows.iter().map(|(n, v)| mrow(n, v)).collect()),
                ..Default::default()
            }),
        }
    }

    /// A controller's moods in `doc`: (name, SID, rendered values), off mood included.
    fn moods_in(doc: &LoxoneDoc, uuid: &str) -> Vec<(String, String, String)> {
        let m = loxone_config::moods::Moods::read(find(&doc.xml.root, uuid).unwrap()).unwrap();
        m.moods
            .iter()
            .map(|x| {
                (
                    x.name.clone(),
                    x.sid.clone().unwrap_or_default(),
                    loxone_config::moods::render(&x.qs),
                )
            })
            .collect()
    }

    fn mood_ctx<'a>(
        d: &'a LoxoneDoc,
        lock: &'a Lock,
        names: &'a BTreeSet<String>,
        claimed: &'a crate::set_claim::Claimed,
    ) -> Ctx<'a> {
        Ctx {
            doc: d,
            lock,
            base: d,
            deployed: None,
            names,
            lets: &NO_LETS,
            drawn: &EMPTY_WIRES,
            adoptions: &EMPTY_ADOPT,
            page_adoptions: &EMPTY_PAGES,
            claimed,
            templates: &NO_TEMPLATES,
            module: &crate::module_edit::NO_MODULE,
        }
    }

    static EMPTY_WIRES: BTreeSet<Wire> = BTreeSet::new();
    static EMPTY_ADOPT: BTreeMap<String, crate::adopt::Adoption> = BTreeMap::new();
    static EMPTY_PAGES: BTreeMap<String, Vec<String>> = BTreeMap::new();

    #[test]
    fn moods_are_a_set_lox_writes_onto_the_controller() {
        use crate::mood_edit::fixture::{FLUR, LICHT};
        let d = LoxoneDoc::parse(crate::mood_edit::fixture::house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = mood_ctx(&d, &lock, &names, &claimed);
        let mut files = Files::default();
        // "Kino" deleted, "Neu" added, "Lesen" first.
        let op = moods_op(
            LICHT,
            None,
            &[
                ("Lesen", "AQ1 on, AQ2 50%"),
                ("Viel Licht", "all on"),
                ("Neu", "AQ3 temp(80%, 3000K)"),
            ],
        );
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        assert_eq!(a.previous, serde_json::Value::Null);
        let text = &files.render()["edits.loxl"];
        let s = slug(LICHT);
        for want in [
            format!("extern {s} = LightController2(uuid: \"{LICHT}\")\n"),
            "moods vx_licht_moods\n\t\"Lesen\": AQ1 on, AQ2 50%\n\t\"Viel Licht\": all on\n\t\"Neu\": AQ3 temp(80%, 3000K)\nend\n"
                .to_string(),
            format!("{s}.moods = vx_licht_moods\n"),
        ] {
            assert!(text.contains(&want), "{want} in\n{text}");
        }
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().claims,
            files.claims
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        let got = moods_in(&out, LICHT);
        // The off mood stays last; Lesen keeps its scene id, and Neu takes the
        // lowest free one: Kino's, which a button on Sel2 would now select.
        assert_eq!(
            got.iter()
                .map(|m| (m.0.as_str(), m.1.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Lesen", "1"),
                ("Viel Licht", "777"),
                ("Neu", "2"),
                ("Aus", "778")
            ]
        );
        assert_eq!(got[0].2, "AQ1 on, AQ2 50%");
        // `all on` fits switches only: the dimmer and the white circuit stay off.
        assert_eq!(got[1].2, "all on, AQ2 off, AQ3 off");
        assert!(crate::mood_edit::missed(&files, &out, &lock, &[LICHT.into()], &[]).is_empty());
        assert!(crate::mood_edit::missed(&files, &d, &lock, &[LICHT.into()], &[]) == ["Licht"]);
        assert!(crate::mood_edit::differs(&out, &d, LICHT));
        assert!(!crate::mood_edit::differs(&out, &d, FLUR));

        // Changed again: its own set is rewritten; previous is what it claimed.
        let ctx2 = Ctx { doc: &out, ..ctx };
        let op2 = moods_op(LICHT, None, &[("Viel Licht", "all off")]);
        let a2 = apply_op(&mut files, &ctx2, &op2).unwrap();
        let prev: crate::mood_edit::MoodsVal =
            serde_json::from_value::<Option<_>>(a2.previous.clone())
                .unwrap()
                .unwrap();
        assert_eq!(prev.set.as_deref(), Some("vx_licht_moods"));
        assert_eq!(prev.moods.as_ref().unwrap().len(), 3);
        assert_eq!(files.claims.3.sets.len(), 1);
        let undo = undo_ops(std::slice::from_ref(&op2), &[a2.previous], &[]).unwrap();
        assert_eq!(
            undo,
            [Op::Moods {
                uuid: LICHT.into(),
                value: Some(prev)
            }]
        );
        // Dropped: the controller is as the base has it; the set and extern go.
        apply_op(
            &mut files,
            &ctx2,
            &Op::Moods {
                uuid: LICHT.into(),
                value: None,
            },
        )
        .unwrap();
        assert!(files.claims.3.is_empty() && files.claims.0.is_empty());
        let out3 = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(moods_in(&out3, LICHT), moods_in(&d, LICHT));
    }

    #[test]
    fn moods_lox_would_refuse_are_refused_first() {
        use crate::mood_edit::fixture::{BAD, LICHT};
        let d = LoxoneDoc::parse(crate::mood_edit::fixture::house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let lock = Lock::default();
        let ctx = mood_ctx(&d, &lock, &names, &claimed);
        let err = |op: Op| apply_op(&mut Files::default(), &ctx, &op).unwrap_err();
        // "Viel Licht" is built in: it stays (a rename is a delete).
        let e = err(moods_op(LICHT, None, &[("Lesen", "all on")]));
        assert!(e.contains("\"Viel Licht\" is a built-in mood"), "{e}");
        let e = err(moods_op(
            LICHT,
            None,
            &[("Bright", "all on"), ("Lesen", "")],
        ));
        assert!(e.contains("cannot be deleted or renamed"), "{e}");
        // The off mood is never in the set.
        let e = err(moods_op(
            LICHT,
            None,
            &[("Viel Licht", "all on"), ("Aus", "")],
        ));
        assert!(e.contains("off mood"), "{e}");
        // A value the circuit cannot take: a colour on a switch, on on a dimmer.
        let e = err(moods_op(LICHT, None, &[("Viel Licht", "AQ1 rgb(1, 2, 3)")]));
        assert!(
            e.contains("AQ1 is") && e.contains("switch or dimmer"),
            "{e}"
        );
        let e = err(moods_op(BAD, None, &[("Viel Licht", "AQ1 on")]));
        assert!(
            e.starts_with("Bad: ") && e.contains("dimmer circuit"),
            "{e}"
        );
        let e = err(moods_op(LICHT, None, &[("Viel Licht", "AQ19 on")]));
        assert!(e.contains("18 circuits"), "{e}");
        let e = err(moods_op(LICHT, None, &[("Viel Licht", "AQ1 bright")]));
        assert!(e.contains("mood \"Viel Licht\""), "{e}");
        // `all on` sets only the circuits it fits: fine on a dimmer.
        apply_op(
            &mut Files::default(),
            &ctx,
            &moods_op(BAD, None, &[("Viel Licht", "all on")]),
        )
        .unwrap();
        // Not a controller; one the module claims.
        assert!(err(moods_op(DIV, None, &[("A", "")])).contains("only a lighting controller"));
        let elsewhere = BTreeMap::from([(
            "moods",
            BTreeMap::from([(LICHT.to_string(), "pages/licht.loxl:4".to_string())]),
        )]);
        let e = apply_op(
            &mut Files::default(),
            &mood_ctx(&d, &lock, &names, &elsewhere),
            &moods_op(LICHT, None, &[("Viel Licht", "all on")]),
        )
        .unwrap_err();
        assert!(e.contains("pages/licht.loxl:4 claims its moods"), "{e}");
        // Dropping vislox's claim on it is always let through.
        apply_op(
            &mut Files::default(),
            &mood_ctx(&d, &lock, &names, &elsewhere),
            &Op::Moods {
                uuid: LICHT.into(),
                value: None,
            },
        )
        .unwrap();
    }

    /// Rewriting a set other controllers share is undone whole, and a
    /// controller whose circuits do not take the set is refused (#74).
    #[test]
    fn a_shared_mood_set_is_undone_whole_and_checked_for_every_controller() {
        use crate::mood_edit::fixture::{BAD, FLUR, LICHT};
        let d = LoxoneDoc::parse(crate::mood_edit::fixture::house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = mood_ctx(&d, &lock, &names, &claimed);
        let mut files = Files::default();
        apply_op(
            &mut files,
            &ctx,
            &moods_op(
                LICHT,
                None,
                &[("Viel Licht", "all on"), ("Lesen", "AQ1 on")],
            ),
        )
        .unwrap();
        // Flur takes the same set; Bad cannot (AQ1 on does not fit its dimmer).
        apply_op(
            &mut files,
            &ctx,
            &moods_op(FLUR, Some("vx_licht_moods"), &[]),
        )
        .unwrap();
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &moods_op(BAD, Some("vx_licht_moods"), &[]),
        )
        .unwrap_err();
        assert!(e.contains("Bad") && e.contains("dimmer circuit"), "{e}");
        let was = files.clone();
        let licht_before = moods_in(&compile_files(&d, &files, &mut l.clone()).unwrap(), LICHT);
        // Flur rewrites the shared set: both change; the undo gives Licht its moods back.
        let op = moods_op(FLUR, Some("vx_licht_moods"), &[("Viel Licht", "AQ2 40%")]);
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(moods_in(&out, LICHT)[0].2, "AQ2 40%");
        assert_eq!(moods_in(&out, FLUR)[0].2, "AQ2 40%");
        let undo = undo_ops(
            std::slice::from_ref(&op),
            std::slice::from_ref(&a.previous),
            &[],
        )
        .unwrap();
        let Op::Moods { value: Some(v), .. } = &undo[0] else {
            panic!("{undo:?}")
        };
        assert_eq!(v.set.as_deref(), Some("vx_licht_moods"));
        let b = apply_op(&mut files, &ctx, &undo[0]).unwrap();
        assert_eq!(files.claims, was.claims);
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(moods_in(&out, LICHT), licht_before);
        // Flur on a set of its own: Licht's stays; and the redo rewrites it whole again.
        let redo = undo_ops(std::slice::from_ref(&undo[0]), &[b.previous], &[]).unwrap();
        apply_op(&mut files, &ctx, &redo[0]).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(moods_in(&out, LICHT)[0].2, "AQ2 40%");
        apply_op(
            &mut files,
            &ctx,
            &moods_op(FLUR, None, &[("Viel Licht", "all off")]),
        )
        .unwrap();
        assert_eq!(files.claims.3.sets.len(), 2);
        assert_eq!(files.claims.3.claims[&slug(FLUR)], "vx_flur_moods");
        // A mood set and a schedule set share one namespace.
        assert!(crate::set_claim::taken(&files, &ctx, "vx_flur_moods"));
    }

    #[test]
    fn a_moods_claim_vislox_did_not_write_is_refused() {
        use crate::mood_edit::fixture::LICHT;
        let s = slug(LICHT);
        let ext = format!("extern {s} = LightController2(uuid: \"{LICHT}\")\n");
        let set = "moods vx_a\n\t\"Viel Licht\": all on\nend\n";
        let read = |t: String| Files::from_texts(&BTreeMap::from([("edits.loxl".to_string(), t)]));
        assert!(read(format!("{ext}{set}{s}.moods = vx_a\n")).is_ok());
        assert!(
            read(format!(
                "{ext}moods wohnen\n\t\"A\": all on\nend\n{s}.moods = wohnen\n"
            ))
            .is_err()
        );
        assert!(read(format!("{ext}{s}.moods = vx_b\n")).is_err());
        assert!(
            read(format!("{set}{s}.moods = vx_a\n"))
                .unwrap_err()
                .contains("neither an extern")
        );
    }

    /// `Circuit<n>:` on a controller vislox added: switch or dimmer, never on
    /// a colour circuit (#74).
    #[test]
    fn a_circuit_is_declared_as_switch_or_dimmer_on_a_managed_controller() {
        use crate::mood_edit::{circuit_setting, fixture::LICHT};
        let d = LoxoneDoc::parse(crate::mood_edit::fixture::house().as_bytes()).unwrap();
        let el = find(&d.xml.root, LICHT).unwrap();
        assert!(circuit_setting(el, "Circuit2", "dimmer").is_ok());
        assert!(
            circuit_setting(el, "Circuit3", "dimmer")
                .unwrap_err()
                .contains("colour circuit")
        );
        assert!(
            circuit_setting(el, "Circuit1", "rgb")
                .unwrap_err()
                .contains("\"switch\" or \"dimmer\"")
        );
        assert!(circuit_setting(el, "Circuit6", "switch").is_err());
        // Written into the declaration of a controller added here.
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let lock0 = lock_of(&l);
        let ctx = mood_ctx(&d, &lock0, &names, &claimed);
        apply_op(
            &mut files,
            &ctx,
            &add(OG, "LightController2", Some("Neu"), &[], &[]),
        )
        .unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        let (slug_new, uuid) = l
            .objects
            .iter()
            .find(|(s, _)| s.starts_with("vx_lightcontroller2"))
            .map(|(s, o)| (s.clone(), o.uuid.clone()))
            .unwrap();
        let ctx = Ctx {
            doc: &out,
            ..mood_ctx(&d, &lock, &names, &claimed)
        };
        let op = Op::Setting {
            uuid: uuid.clone(),
            name: "Circuit1".into(),
            value: Some("dimmer".into()),
        };
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        assert_eq!(a.previous, serde_json::Value::Null);
        let text = files
            .render()
            .into_iter()
            .find(|(k, _)| k != "edits.loxl")
            .unwrap()
            .1;
        assert!(
            text.contains(&format!(
                "{slug_new} = LightController2(\"Neu\", Circuit1: \"dimmer\")"
            )),
            "{text}"
        );
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &Op::Setting {
                uuid,
                name: "Circuit1".into(),
                value: Some("rgb".into()),
            },
        )
        .unwrap_err();
        assert!(e.contains("\"switch\" or \"dimmer\""), "{e}");
    }

    /// Changing a circuit's kind on a controller whose moods vislox claims
    /// re-checks the claimed set: a value the new kind does not take is
    /// refused up front, naming the mood, not by lox's compile (#74).
    #[test]
    fn a_circuit_kind_the_claimed_moods_do_not_fit_is_refused() {
        let d = LoxoneDoc::parse(crate::mood_edit::fixture::house().as_bytes()).unwrap();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let lock0 = lock_of(&l);
        let ctx = mood_ctx(&d, &lock0, &names, &claimed);
        apply_op(
            &mut files,
            &ctx,
            &add(OG, "LightController2", Some("Neu"), &[], &[]),
        )
        .unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        let uuid = l
            .objects
            .iter()
            .find(|(s, _)| s.starts_with("vx_lightcontroller2"))
            .map(|(_, o)| o.uuid.clone())
            .unwrap();
        let ctx = Ctx {
            doc: &out,
            ..mood_ctx(&d, &lock, &names, &claimed)
        };
        // Its moods claimed: AQ1 on (a switch's value), AQ2 40%.
        apply_op(
            &mut files,
            &ctx,
            &moods_op(&uuid, None, &[("Viel Licht", "AQ1 on, AQ2 40%")]),
        )
        .unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let ctx = Ctx {
            doc: &out,
            ..mood_ctx(&d, &lock, &names, &claimed)
        };
        let set = |n: &str, v: &str| Op::Setting {
            uuid: uuid.clone(),
            name: n.into(),
            value: Some(v.into()),
        };
        let e = apply_op(&mut files.clone(), &ctx, &set("Circuit1", "dimmer")).unwrap_err();
        assert!(
            e.contains("do not fit AQ1 as a dimmer")
                && e.contains("mood \"Viel Licht\": AQ1 is `on`")
                && e.contains("Edit moods"),
            "{e}"
        );
        // A dimmer takes the 40% on AQ2; the claim stays.
        apply_op(&mut files, &ctx, &set("Circuit2", "dimmer")).unwrap();
        assert_eq!(files.claims.3.claims.len(), 1);
        // Not claimed: nothing to re-check.
        let mut free = files.clone();
        apply_op(
            &mut free,
            &ctx,
            &Op::Moods {
                uuid: uuid.clone(),
                value: None,
            },
        )
        .unwrap();
        apply_op(&mut free, &ctx, &set("Circuit1", "dimmer")).unwrap();
    }

    /// A claimant of a shared set that the pending config lost (deleted in
    /// Loxone Config) is refused by name, not skipped; its stale claim is
    /// dropped with the op and no value (#74).
    #[test]
    fn a_shared_set_whose_claimant_is_gone_is_refused_until_its_claim_is_dropped() {
        use crate::mood_edit::fixture::{FLUR, LICHT};
        let full = crate::mood_edit::fixture::house();
        let d = LoxoneDoc::parse(full.as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let lock = Lock::default();
        let ctx = mood_ctx(&d, &lock, &names, &claimed);
        let mut files = Files::default();
        apply_op(
            &mut files,
            &ctx,
            &moods_op(LICHT, None, &[("Viel Licht", "all on")]),
        )
        .unwrap();
        apply_op(
            &mut files,
            &ctx,
            &moods_op(FLUR, Some("vx_licht_moods"), &[]),
        )
        .unwrap();
        // Flur deleted in Loxone Config.
        let a = full
            .find(&format!(
                "<C Type=\"LightController2\" V=\"178\" U=\"{FLUR}\""
            ))
            .unwrap();
        let b = a + full[a..].find("</C>\n").unwrap() + 5;
        let gone = LoxoneDoc::parse(format!("{}{}", &full[..a], &full[b..]).as_bytes()).unwrap();
        let ctx = mood_ctx(&gone, &lock, &names, &claimed);
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &moods_op(LICHT, Some("vx_licht_moods"), &[("Viel Licht", "AQ1 on")]),
        )
        .unwrap_err();
        assert!(
            e.contains(&format!(
                "also claimed for {FLUR}, which the pending config does not have"
            )) && e.contains("drop that stale claim"),
            "{e}"
        );
        // The section of GET /api/edits names it.
        let (_, sets) = crate::set_claim::edits_section::<crate::mood_edit::MoodKind>(
            &files,
            &Files::default(),
            &lock,
            Some(&gone),
            None,
        );
        assert_eq!(sets[0]["stale"], serde_json::json!([FLUR]));
        // Dropped, Licht's set is written again.
        apply_op(
            &mut files,
            &ctx,
            &Op::Moods {
                uuid: FLUR.into(),
                value: None,
            },
        )
        .unwrap();
        apply_op(
            &mut files,
            &ctx,
            &moods_op(LICHT, Some("vx_licht_moods"), &[("Viel Licht", "AQ1 on")]),
        )
        .unwrap();
        assert_eq!(files.claims.3.claimants("vx_licht_moods"), [slug(LICHT)]);
        // A block the config does not have takes no new claim.
        assert!(
            apply_op(
                &mut files,
                &ctx,
                &moods_op(FLUR, None, &[("Viel Licht", "")])
            )
            .unwrap_err()
            .contains("no object")
        );
    }

    // ── access rights (#76) ────────────────────────────────────────────

    fn access_op(uuid: &str, set: Option<&str>, grants: &[(&str, &str, &str)]) -> Op {
        Op::Access {
            uuid: uuid.into(),
            value: Some(crate::access_edit::AccessVal {
                set: set.map(String::from),
                grants: (set.is_none() || !grants.is_empty()).then(|| {
                    grants
                        .iter()
                        .map(|(k, u, s)| crate::access_edit::Grant {
                            kind: k.to_string(),
                            uuid: u.to_string(),
                            schedule: s.to_string(),
                        })
                        .collect()
                }),
                restore: BTreeMap::new(),
            }),
        }
    }

    fn access_ctx<'a>(
        d: &'a LoxoneDoc,
        lock: &'a Lock,
        names: &'a BTreeSet<String>,
        claimed: &'a crate::set_claim::Claimed,
    ) -> Ctx<'a> {
        Ctx {
            doc: d,
            lock,
            base: d,
            deployed: None,
            names,
            lets: &NO_LETS,
            drawn: &EMPTY_WIRES,
            adoptions: &EMPTY_ADOPT,
            page_adoptions: &EMPTY_PAGES,
            claimed,
            templates: &NO_TEMPLATES,
            module: &crate::module_edit::NO_MODULE,
        }
    }

    /// Who may open `door` in `doc`: (title, schedule title).
    fn opens(doc: &LoxoneDoc, door: &str) -> Vec<(String, String)> {
        crate::rights::read(doc)[door]
            .list
            .iter()
            .map(|p| (p.title.clone(), p.schedule.clone().unwrap_or_default()))
            .collect()
    }

    #[test]
    fn access_is_a_set_lox_writes_onto_the_code_touch() {
        use crate::access_edit::fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = access_ctx(&d, &lock, &names, &claimed);
        let mut files = Files::default();
        // alice is left out; the group may open the door on workdays.
        let op = access_op(DOOR, None, &[("group", BEWOHNER, WORKDAYS)]);
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        assert_eq!(a.previous, serde_json::Value::Null);
        let text = &files.render()["edits.loxl"];
        let s = slug(DOOR);
        for want in [
            format!("extern {s} = NfcCodeTouch(uuid: \"{DOOR}\")\n"),
            "access vx_haustuer_access\n\tgroup \"Bewohner\": \"Workdays\"\nend\n".to_string(),
            format!("{s}.access = vx_haustuer_access\n"),
        ] {
            assert!(text.contains(&want), "{want} in\n{text}");
        }
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().claims,
            files.claims
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(
            opens(&out, DOOR),
            [("Bewohner".to_string(), "Workdays".to_string())]
        );
        // alice's ACT= (operating the block in the app) is no access right: kept.
        let xml = String::from_utf8(out.to_bytes()).unwrap();
        assert!(
            xml.contains(&format!("<PRM REF=\"{DOOR}\" ACT=\"3\"/>")),
            "{}",
            &xml[xml.find("Title=\"alice\"").unwrap()..][..400]
        );
        // The review, and Apply's confirmation, say who loses and who gains it.
        let ch =
            crate::rights::changes(&crate::rights::read(&d), &crate::rights::read(&out), |u| {
                u.to_string()
            });
        let door = ch.iter().find(|c| c.uuid == DOOR).unwrap();
        assert_eq!(door.lose[0].title, "alice");
        assert_eq!(door.gain[0].title, "bob");
        assert!(crate::access_edit::missed(&files, &out, &lock, &[DOOR.into()], &[]).is_empty());
        assert_eq!(
            crate::access_edit::missed(&files, &d, &lock, &[DOOR.into()], &[]),
            ["Haustür"]
        );
        assert!(crate::access_edit::differs(&out, &d, DOOR));
        assert!(!crate::access_edit::differs(&out, &d, GARAGE));

        // Changed again: its own set is rewritten; previous is what it claimed,
        // by UUID (lowercase), so the undo gives it back exactly.
        let ctx2 = Ctx { doc: &out, ..ctx };
        let op2 = access_op(DOOR, None, &[]);
        let a2 = apply_op(&mut files, &ctx2, &op2).unwrap();
        let prev: crate::access_edit::AccessVal =
            serde_json::from_value::<Option<_>>(a2.previous.clone())
                .unwrap()
                .unwrap();
        assert_eq!(prev.set.as_deref(), Some("vx_haustuer_access"));
        assert_eq!(prev.grants.as_ref().unwrap()[0].uuid, BEWOHNER);
        // An empty set revokes everyone.
        let out2 = compile_files(&d, &files, &mut l).unwrap();
        assert!(opens(&out2, DOOR).is_empty());
        let undo = undo_ops(std::slice::from_ref(&op2), &[a2.previous], &[]).unwrap();
        assert_eq!(
            undo,
            [Op::Access {
                uuid: DOOR.into(),
                value: Some(prev)
            }]
        );
        apply_op(&mut files, &Ctx { doc: &out2, ..ctx2 }, &undo[0]).unwrap();
        assert_eq!(
            opens(&compile_files(&d, &files, &mut l).unwrap(), DOOR)[0].0,
            "Bewohner"
        );
        // Dropped: the set and extern go, and the rights stay as last written.
        apply_op(
            &mut files,
            &ctx2,
            &Op::Access {
                uuid: DOOR.into(),
                value: None,
            },
        )
        .unwrap();
        assert!(files.claims.5.is_empty() && files.claims.0.is_empty());
    }

    #[test]
    fn access_lox_would_refuse_is_refused_first() {
        use crate::access_edit::fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let lock = Lock::default();
        let ctx = access_ctx(&d, &lock, &names, &claimed);
        let err = |op: Op| apply_op(&mut Files::default(), &ctx, &op).unwrap_err();
        let e = err(access_op(
            DOOR,
            None,
            &[("user", BOB, ALWAYS), ("user", BOB, WORKDAYS)],
        ));
        assert!(
            e.starts_with("Haustür: ") && e.contains("listed twice"),
            "{e}"
        );
        let e = err(access_op(DOOR, None, &[("user", TWIN2, ALWAYS)]));
        assert!(e.contains("titled \"chris\""), "{e}");
        // Not a Code Touch; one the module claims.
        assert!(
            err(access_op(DIV, None, &[("user", BOB, ALWAYS)])).contains("only an NFC Code Touch")
        );
        let elsewhere = BTreeMap::from([(
            "access",
            BTreeMap::from([(DOOR.to_string(), "pages/zugang.loxl:7".to_string())]),
        )]);
        let e = apply_op(
            &mut Files::default(),
            &access_ctx(&d, &lock, &names, &elsewhere),
            &access_op(DOOR, None, &[("user", BOB, ALWAYS)]),
        )
        .unwrap_err();
        assert!(
            e.contains("pages/zugang.loxl:7 claims its access rights"),
            "{e}"
        );
        // Dropping vislox's claim on it is always let through.
        apply_op(
            &mut Files::default(),
            &access_ctx(&d, &lock, &names, &elsewhere),
            &Op::Access {
                uuid: DOOR.into(),
                value: None,
            },
        )
        .unwrap();
    }

    /// Two doors on one set: rewriting it through one changes the other, the
    /// review says so for both, and the undo gives the other its rights back.
    #[test]
    fn a_shared_access_set_is_undone_whole() {
        use crate::access_edit::fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = access_ctx(&d, &lock, &names, &claimed);
        let mut files = Files::default();
        apply_op(
            &mut files,
            &ctx,
            &access_op(
                DOOR,
                None,
                &[("user", ALICE, ALWAYS), ("user", BOB, ALWAYS)],
            ),
        )
        .unwrap();
        // Same rights as Haustür.
        apply_op(
            &mut files,
            &ctx,
            &access_op(GARAGE, Some("vx_haustuer_access"), &[]),
        )
        .unwrap();
        let before = compile_files(&d, &files, &mut l.clone()).unwrap();
        assert_eq!(opens(&before, GARAGE).len(), 2);
        let was = files.clone();
        // The garage rewrites the shared set: bob loses both doors.
        let op = access_op(
            GARAGE,
            Some("vx_haustuer_access"),
            &[("user", ALICE, ALWAYS)],
        );
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let ch = crate::rights::changes(
            &crate::rights::read(&before),
            &crate::rights::read(&out),
            |u| u.to_string(),
        );
        for door in [DOOR, GARAGE] {
            let c = ch.iter().find(|c| c.uuid == door).unwrap();
            assert_eq!(
                c.lose.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(),
                ["bob"]
            );
        }
        let undo = undo_ops(
            std::slice::from_ref(&op),
            std::slice::from_ref(&a.previous),
            &[],
        )
        .unwrap();
        apply_op(&mut files, &ctx, &undo[0]).unwrap();
        assert_eq!(files.claims, was.claims);
        // Its own set: the door's stays.
        apply_op(
            &mut files,
            &ctx,
            &access_op(GARAGE, None, &[("user", ALICE, WORKDAYS)]),
        )
        .unwrap();
        assert_eq!(files.claims.5.claims[&slug(GARAGE)], "vx_garage_access");
        assert_eq!(files.claims.5.sets.len(), 2);
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(opens(&out, DOOR).len(), 2);
        assert_eq!(
            opens(&out, GARAGE),
            [("alice".to_string(), "Workdays".to_string())]
        );
        // Access sets share the namespace with the other sets.
        assert!(crate::set_claim::taken(&files, &ctx, "vx_garage_access"));
    }

    #[test]
    fn an_access_claim_vislox_did_not_write_is_refused() {
        use crate::access_edit::fixture::DOOR;
        let s = slug(DOOR);
        let ext = format!("extern {s} = NfcCodeTouch(uuid: \"{DOOR}\")\n");
        let set = "access vx_a\n\tuser \"alice\": \"Always\"\nend\n";
        let read = |t: String| Files::from_texts(&BTreeMap::from([("edits.loxl".to_string(), t)]));
        assert!(read(format!("{ext}{set}{s}.access = vx_a\n")).is_ok());
        assert!(
            read(format!(
                "{ext}access bewohner\n\tuser \"alice\": \"Always\"\nend\n{s}.access = bewohner\n"
            ))
            .is_err()
        );
        assert!(read(format!("{ext}{s}.access = vx_b\n")).is_err());
        assert!(
            read(format!("{set}{s}.access = vx_a\n"))
                .unwrap_err()
                .contains("neither an extern")
        );
        // A recipients set is the recipients kind's (#77), not access's.
        assert!(
            read("recipients vx_r\n\tuser \"alice\"\nend\n".to_string())
                .unwrap()
                .claims
                .5
                .is_empty()
        );
    }

    // ── recipients of a push notification (#77) ───────────────────────

    fn recipients_op(uuid: &str, set: Option<&str>, list: &[(&str, &str)]) -> Op {
        Op::Recipients {
            uuid: uuid.into(),
            value: Some(crate::recipients_edit::RecipientsVal {
                set: set.map(String::from),
                recipients: (set.is_none() || !list.is_empty()).then(|| {
                    list.iter()
                        .map(|(k, u)| crate::recipients_edit::Recipient {
                            kind: k.to_string(),
                            uuid: u.to_string(),
                        })
                        .collect()
                }),
                restore: BTreeMap::new(),
            }),
        }
    }

    /// Who gets `n` in `doc` directly: titles.
    fn gets(doc: &LoxoneDoc, n: &str) -> Vec<String> {
        crate::rights::read(doc)[n]
            .list
            .iter()
            .map(|p| p.title.clone())
            .collect()
    }

    #[test]
    fn recipients_are_a_set_lox_writes_onto_the_notification() {
        use crate::recipients_edit::fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = access_ctx(&d, &lock, &names, &claimed);
        let mut files = Files::default();
        // alice is left out; bob and the family get it.
        let op = recipients_op(FROST, None, &[("user", BOB), ("group", FAMILIE)]);
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        assert_eq!(a.previous, serde_json::Value::Null);
        let text = &files.render()["edits.loxl"];
        let s = slug(FROST);
        for want in [
            format!("extern {s} = Notification(uuid: \"{FROST}\")\n"),
            "recipients vx_frost_recipients\n\tuser \"bob\"\n\tgroup \"Familie\"\nend\n"
                .to_string(),
            format!("{s}.recipients = vx_frost_recipients\n"),
        ] {
            assert!(text.contains(&want), "{want} in\n{text}");
        }
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().claims,
            files.claims
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(gets(&out, FROST), ["bob", "Familie"]);
        // The review, and Apply's confirmation, say who stops getting it;
        // carol keeps it through Full access, inherited.
        let ch =
            crate::rights::changes(&crate::rights::read(&d), &crate::rights::read(&out), |u| {
                u.to_string()
            });
        let frost = ch.iter().find(|c| c.uuid == FROST).unwrap();
        assert_eq!(frost.kind, "recipients");
        assert_eq!(
            frost
                .lose
                .iter()
                .map(|u| u.title.as_str())
                .collect::<Vec<_>>(),
            ["alice"]
        );
        assert_eq!(frost.gain[0].title, "bob");
        assert_eq!(crate::rights::read(&out)[FROST].inherited[0].title, "carol");
        assert!(
            crate::recipients_edit::missed(&files, &out, &lock, &[FROST.into()], &[]).is_empty()
        );
        assert_eq!(
            crate::recipients_edit::missed(&files, &d, &lock, &[FROST.into()], &[]),
            ["Frost"]
        );
        assert!(crate::recipients_edit::differs(&out, &d, FROST));
        assert!(!crate::recipients_edit::differs(&out, &d, REGEN));

        // Emptied: only Full access gets it. previous is the list by UUID.
        let ctx2 = Ctx { doc: &out, ..ctx };
        let op2 = recipients_op(FROST, None, &[]);
        let a2 = apply_op(&mut files, &ctx2, &op2).unwrap();
        let prev: crate::recipients_edit::RecipientsVal =
            serde_json::from_value::<Option<_>>(a2.previous.clone())
                .unwrap()
                .unwrap();
        assert_eq!(prev.set.as_deref(), Some("vx_frost_recipients"));
        assert_eq!(prev.recipients.as_ref().unwrap()[1].uuid, FAMILIE);
        let out2 = compile_files(&d, &files, &mut l).unwrap();
        assert!(gets(&out2, FROST).is_empty());
        assert_eq!(crate::rights::read(&out2)[FROST].users[0].title, "carol");
        let undo = undo_ops(std::slice::from_ref(&op2), &[a2.previous], &[]).unwrap();
        assert_eq!(
            undo,
            [Op::Recipients {
                uuid: FROST.into(),
                value: Some(prev)
            }]
        );
        apply_op(&mut files, &Ctx { doc: &out2, ..ctx2 }, &undo[0]).unwrap();
        assert_eq!(
            gets(&compile_files(&d, &files, &mut l).unwrap(), FROST),
            ["bob", "Familie"]
        );
        // Regen shares Frost's set: both get the family; the undo of a rewrite
        // through Frost gives Regen its recipients back.
        apply_op(
            &mut files,
            &ctx2,
            &recipients_op(REGEN, Some("vx_frost_recipients"), &[]),
        )
        .unwrap();
        let op3 = recipients_op(FROST, Some("vx_frost_recipients"), &[("user", ALICE)]);
        let a3 = apply_op(&mut files, &ctx2, &op3).unwrap();
        let out3 = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(gets(&out3, REGEN), ["alice"]);
        let undo = undo_ops(std::slice::from_ref(&op3), &[a3.previous], &[]).unwrap();
        apply_op(&mut files, &Ctx { doc: &out3, ..ctx2 }, &undo[0]).unwrap();
        assert_eq!(
            gets(&compile_files(&d, &files, &mut l).unwrap(), REGEN),
            ["bob", "Familie"]
        );
        // Recipient sets share the namespace with the other sets.
        assert!(crate::set_claim::taken(
            &files,
            &ctx2,
            "vx_frost_recipients"
        ));
        // Dropped: the sets and externs go, and the recipients stay as last written.
        for n in [FROST, REGEN] {
            apply_op(
                &mut files,
                &ctx2,
                &Op::Recipients {
                    uuid: n.into(),
                    value: None,
                },
            )
            .unwrap();
        }
        assert!(files.claims.6.is_empty() && files.claims.0.is_empty());
    }

    #[test]
    fn recipients_lox_would_refuse_are_refused_first() {
        use crate::recipients_edit::fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let lock = Lock::default();
        let ctx = access_ctx(&d, &lock, &names, &claimed);
        let err = |op: Op| apply_op(&mut Files::default(), &ctx, &op).unwrap_err();
        let e = err(recipients_op(FROST, None, &[("user", BOB), ("user", BOB)]));
        assert!(
            e.starts_with("Frost: ") && e.contains("listed twice"),
            "{e}"
        );
        let e = err(recipients_op(FROST, None, &[("user", TWIN2)]));
        assert!(e.contains("titled \"chris\""), "{e}");
        // The notification's block on a page is no notification: the claim is on
        // the Notification itself.
        assert!(
            err(recipients_op(WARN, None, &[("user", BOB)])).contains("only a push notification")
        );
        let elsewhere = BTreeMap::from([(
            "recipients",
            BTreeMap::from([(FROST.to_string(), "pages/meldungen.loxl:9".to_string())]),
        )]);
        let e = apply_op(
            &mut Files::default(),
            &access_ctx(&d, &lock, &names, &elsewhere),
            &recipients_op(FROST, None, &[("user", BOB)]),
        )
        .unwrap_err();
        assert!(
            e.contains("pages/meldungen.loxl:9 claims its recipients"),
            "{e}"
        );
        // Dropping vislox's claim on it is always let through.
        apply_op(
            &mut Files::default(),
            &access_ctx(&d, &lock, &names, &elsewhere),
            &Op::Recipients {
                uuid: FROST.into(),
                value: None,
            },
        )
        .unwrap();
    }

    // ── tiles of a Touch Pure Flex (#78) ──────────────────────────────

    fn tf(api: &str, text: Option<&str>, to: &[&str]) -> Option<crate::tiles_edit::FnVal> {
        Some(crate::tiles_edit::FnVal {
            api: api.into(),
            text: text.map(String::from),
            to: to.iter().map(|x| x.to_string()).collect(),
        })
    }

    fn tiles_op(
        uuid: &str,
        set: Option<&str>,
        tiles: Option<Vec<crate::tiles_edit::TileVal>>,
    ) -> Op {
        Op::Tiles {
            uuid: uuid.into(),
            value: Some(crate::tiles_edit::TilesVal {
                set: set.map(String::from),
                tiles,
                restore: BTreeMap::new(),
            }),
        }
    }

    fn tile(
        index: u32,
        click: Option<crate::tiles_edit::FnVal>,
        up: Option<crate::tiles_edit::FnVal>,
    ) -> crate::tiles_edit::TileVal {
        crate::tiles_edit::TileVal {
            index,
            click,
            up,
            down: None,
        }
    }

    /// The device's tile `i` in `doc`.
    fn tile_of(doc: &LoxoneDoc, dev: &str, i: u32) -> crate::tiles::Tile {
        crate::tiles::tiles_of(crate::tiles::find_ci(&doc.xml.root, dev).unwrap())
            .unwrap()
            .into_iter()
            .find(|t| t.index == i)
            .unwrap()
    }

    #[test]
    fn tiles_are_a_set_lox_writes_onto_the_device() {
        use crate::tiles::fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = access_ctx(&d, &lock, &names, &claimed);
        let mut files = Files::default();
        // Tile 3: the click closes Süd, the arrow up opens Nord and Süd; tile 2 cleared.
        let op = tiles_op(
            FLEX,
            None,
            Some(vec![
                tile(
                    3,
                    tf("SET(As;Cc;pulse)", Some("=ECHO(As)"), &[SUED]),
                    tf(
                        "SET(As;Co;pulse)",
                        None,
                        &[NORD, &SUED.to_ascii_uppercase()],
                    ),
                ),
                tile(2, None, None),
            ]),
        );
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        assert_eq!(a.previous, serde_json::Value::Null);
        let text = &files.render()["edits.loxl"];
        let (s, n, su) = (slug(FLEX), slug(NORD), slug(SUED));
        for want in [
            format!("extern {s} = TreeDevice(uuid: \"{FLEX}\")\n"),
            format!("extern {n} = AutoJalousie(uuid: \"{NORD}\")\n"),
            format!("extern {su} = AutoJalousie(uuid: \"{SUED}\")\n"),
            "tiles vx_flex_kueche_tiles\n\ttile 2()\n\ttile 3(\n".to_string(),
            format!("\t\tclick_to: {su},\n\t\tup_to: {n},\n\t\tup_to: {su},\n"),
            format!("{s}.tiles = vx_flex_kueche_tiles\n"),
        ] {
            assert!(text.contains(&want), "{want} in\n{text}");
        }
        // One extern per block, whatever the case the op names it in.
        assert_eq!(text.matches(&format!("extern {su} ")).count(), 1);
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().claims,
            files.claims
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        let t3 = tile_of(&out, FLEX, 3);
        assert_eq!(t3.functions[0].api.as_deref(), Some("SET(As;Cc;pulse)"));
        assert_eq!(t3.functions[0].text.as_deref(), Some("=ECHO(As)"));
        assert_eq!(crate::member_edit::lower(&t3.functions[0].ctrls), [SUED]);
        assert_eq!(
            crate::member_edit::lower(&t3.functions[1].ctrls),
            [NORD, SUED]
        );
        assert_eq!(t3.functions[2], Default::default());
        // Tile 5 is not in the set: it keeps its program.
        assert_eq!(tile_of(&out, FLEX, 5), tile_of(&d, FLEX, 5));
        assert!(crate::tiles_edit::missed(&files, &out, &lock, &[FLEX.into()], &[]).is_empty());
        assert_eq!(
            crate::tiles_edit::missed(&files, &d, &lock, &[FLEX.into()], &[]),
            ["Flex Küche"]
        );
        assert!(crate::tiles::differ(&out, &d, FLEX));

        // previous gives the programs back by UUID, and the undo writes them.
        let ctx2 = Ctx { doc: &out, ..ctx };
        let op2 = tiles_op(FLEX, None, Some(vec![tile(3, None, None)]));
        let a2 = apply_op(&mut files, &ctx2, &op2).unwrap();
        let prev: crate::tiles_edit::TilesVal =
            serde_json::from_value::<Option<_>>(a2.previous.clone())
                .unwrap()
                .unwrap();
        assert_eq!(prev.set.as_deref(), Some("vx_flex_kueche_tiles"));
        assert_eq!(
            prev.tiles.as_ref().unwrap()[1].up.as_ref().unwrap().to,
            [NORD, SUED]
        );
        // The arrow's targets are gone, and their externs with them.
        assert!(!files.render()["edits.loxl"].contains(&format!("extern {n} ")));
        let out2 = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(
            tile_of(&out2, FLEX, 3).functions,
            <[crate::tiles::Function; 3]>::default()
        );
        let undo = undo_ops(std::slice::from_ref(&op2), &[a2.previous], &[]).unwrap();
        assert_eq!(
            undo,
            [Op::Tiles {
                uuid: FLEX.into(),
                value: Some(prev)
            }]
        );
        apply_op(&mut files, &Ctx { doc: &out2, ..ctx2 }, &undo[0]).unwrap();
        let out3 = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(tile_of(&out3, FLEX, 3), t3);
        // A tile set shares the namespace with the other sets.
        assert!(crate::set_claim::taken(
            &files,
            &ctx2,
            "vx_flex_kueche_tiles"
        ));
        // Remove a target's extern? It stays while the set names it.
        assert_eq!(
            files.claims.claim_on(&n).map(|x| x.0),
            Some("it in its tiles set `vx_flex_kueche_tiles`".to_string())
        );
        // Dropped: the set and every extern go, and the tiles stay as last written.
        let a4 = apply_op(
            &mut files,
            &ctx2,
            &Op::Tiles {
                uuid: FLEX.into(),
                value: None,
            },
        )
        .unwrap();
        assert!(
            files.claims.7.is_empty() && files.claims.0.is_empty(),
            "{:?}",
            files.claims
        );
        assert!(a4.previous["tiles"].is_array());
    }

    #[test]
    fn tiles_lox_would_refuse_are_refused_first() {
        use crate::tiles::fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let lock = Lock::default();
        let ctx = access_ctx(&d, &lock, &names, &claimed);
        let err = |op: Op| apply_op(&mut Files::default(), &ctx, &op).unwrap_err();
        let one = |t: crate::tiles_edit::TileVal| tiles_op(FLEX, None, Some(vec![t]));
        // A tile the layout does not have.
        let e = err(one(tile(7, None, None)));
        assert!(e.contains("has no tile 7 (it has 2, 3, 5)"), "{e}");
        // A target not linked to the device: Ost is wired to nothing.
        let e = err(one(tile(3, tf("SET(As;Tg;pulse)", None, &[OST]), None)));
        assert!(e.contains("Ost is not API-linked to Flex Küche"), "{e}");
        // A shade command onto the lighting controller.
        let e = err(one(tile(3, tf("SET(As;Tg;pulse)", None, &[LIGHT]), None)));
        assert!(
            e.contains(
                "Licht is a LightController2, but `SET(As;Tg;pulse)` addresses AutoJalousie"
            ),
            "{e}"
        );
        // A command lox does not know the abbreviation of takes any linked target.
        apply_op(
            &mut Files::default(),
            &ctx,
            &one(tile(3, tf("SET(Xy;Tg)", None, &[LIGHT]), None)),
        )
        .unwrap();
        let e = err(one(tile(3, tf("  ", None, &[]), None)));
        assert!(e.contains("give a command"), "{e}");
        let e = err(one(tile(3, tf("SET(As;Tg)\nX", None, &[]), None)));
        assert!(e.contains("is one line"), "{e}");
        let e = err(one(tile(
            3,
            tf("SET(As;Tg)", None, &[NORD, &NORD.to_ascii_uppercase()]),
            None,
        )));
        assert!(e.contains("listed twice"), "{e}");
        let e = err(tiles_op(
            FLEX,
            None,
            Some(vec![tile(3, None, None), tile(3, None, None)]),
        ));
        assert!(e.contains("tile 3 is listed twice"), "{e}");
        // A Touch Tree is a Tree device without a tile layout.
        let e = err(tiles_op(TOUCH, None, Some(vec![tile(3, None, None)])));
        assert!(e.contains("no Touch Pure Flex tile layout"), "{e}");
        assert!(err(tiles_op(NORD, None, Some(vec![]))).contains("only a Touch Pure Flex"));
        // The module claims its tiles.
        let elsewhere = BTreeMap::from([(
            "tiles",
            BTreeMap::from([(FLEX.to_string(), "pages/flex.loxl:4".to_string())]),
        )]);
        let e = apply_op(
            &mut Files::default(),
            &access_ctx(&d, &lock, &names, &elsewhere),
            &one(tile(2, None, None)),
        )
        .unwrap_err();
        assert!(e.contains("pages/flex.loxl:4 claims its tiles"), "{e}");
        // Dropping vislox's claim is always let through.
        apply_op(
            &mut Files::default(),
            &access_ctx(&d, &lock, &names, &elsewhere),
            &Op::Tiles {
                uuid: FLEX.into(),
                value: None,
            },
        )
        .unwrap();
        // An extern of another type for the target's UUID is never overwritten.
        let mut files = Files::default();
        files.claims.0.insert(
            NORD.to_ascii_uppercase(),
            Claim {
                ty: "Pushbutton".into(),
                pinned: true,
                ..Default::default()
            },
        );
        let e = apply_op(
            &mut files,
            &ctx,
            &one(tile(3, tf("SET(As;Tg)", None, &[NORD]), None)),
        )
        .unwrap_err();
        assert!(e.contains("as a Pushbutton, not a AutoJalousie"), "{e}");
    }

    /// A block vislox added is a target by its slug: before lox pins it
    /// (its link is lox's to check, its type is checked here, and `previous`
    /// carries the slug), and after (resolved through the lock).
    #[test]
    fn a_block_added_here_is_a_target_by_its_slug_pinned_or_not() {
        use crate::set_claim::SetKind;
        use crate::tiles::fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let mut l = Lockfile::default();
        let lock0 = lock_of(&l);
        let ctx = access_ctx(&d, &lock0, &names, &claimed);
        let mut files = Files::default();
        apply_op(
            &mut files,
            &ctx,
            &add(PAGE, "LightController2", Some("Neu"), &[], &[]),
        )
        .unwrap();
        let new = files
            .pages
            .values()
            .flat_map(|p| &p.blocks)
            .next()
            .unwrap()
            .slug
            .clone();
        let lico = |to: &str| {
            tiles_op(
                FLEX,
                None,
                Some(vec![tile(5, tf("SET(Lico;M+;pulse)", None, &[to]), None)]),
            )
        };
        // Not pinned: named by its slug, its type checked.
        let mut before = files.clone();
        apply_op(&mut before, &ctx, &lico(&new)).unwrap();
        assert!(
            before.render()["edits.loxl"].contains(&format!("to: {new})")),
            "{:?}",
            before.render()
        );
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &tiles_op(
                FLEX,
                None,
                Some(vec![tile(5, tf("SET(As;Tg;pulse)", None, &[&new]), None)]),
            ),
        )
        .unwrap_err();
        assert!(
            e.contains(&format!(
                "`{new}` is a LightController2, but `SET(As;Tg;pulse)` addresses AutoJalousie"
            )),
            "{e}"
        );
        // The set reads back with the slug, which the undo takes as it is.
        let idx = crate::tiles_edit::TilesKind::index_in(&d, &lock0, &before);
        let v =
            crate::set_claim::current::<crate::tiles_edit::TilesKind>(&before, &slug(FLEX), &idx)
                .unwrap();
        assert_eq!(
            v.tiles.as_ref().unwrap()[0].click.as_ref().unwrap().to,
            std::slice::from_ref(&new)
        );
        let a = apply_op(
            &mut before,
            &ctx,
            &Op::Tiles {
                uuid: FLEX.into(),
                value: None,
            },
        )
        .unwrap();
        let undo = undo_ops(
            &[Op::Tiles {
                uuid: FLEX.into(),
                value: None,
            }],
            &[a.previous],
            &[],
        )
        .unwrap();
        apply_op(&mut before, &ctx, &undo[0]).unwrap();
        assert!(before.render()["edits.loxl"].contains(&format!("to: {new})")));
        // Pinned once compiled: resolved through the lock to its UUID (no longer
        // refused as no object), and refused only as not linked.
        let out = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        assert!(lock.uuid_of(&new).is_some());
        let ctx2 = access_ctx(&out, &lock, &names, &claimed);
        let e = apply_op(&mut files, &ctx2, &lico(&new)).unwrap_err();
        assert!(e.contains("Neu is not API-linked to Flex Küche"), "{e}");
    }

    /// The device's extern is keyed as the file keys it: one `extern` line
    /// whatever the case of the UUID.
    #[test]
    fn the_device_s_extern_is_one_whatever_the_case() {
        use crate::tiles::fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let lock = Lock::default();
        let ctx = access_ctx(&d, &lock, &names, &claimed);
        let mut files = Files::default();
        files.claims.0.insert(
            FLEX.to_ascii_uppercase(),
            Claim {
                ty: "TreeDevice".into(),
                attrs: BTreeMap::from([(Which::Title, "Flex".to_string())]),
                ..Default::default()
            },
        );
        apply_op(
            &mut files,
            &ctx,
            &tiles_op(FLEX, None, Some(vec![tile(2, None, None)])),
        )
        .unwrap();
        let text = &files.render()["edits.loxl"];
        assert_eq!(
            text.matches(&format!("extern {} ", slug(FLEX))).count(),
            1,
            "{text}"
        );
        assert_eq!(files.claims.0.len(), 1);
    }

    #[test]
    fn a_tiles_claim_vislox_did_not_write_is_refused() {
        use crate::tiles::fixture::*;
        let read = |t: String| Files::from_texts(&BTreeMap::from([("edits.loxl".to_string(), t)]));
        let s = slug(FLEX);
        let ext = format!("extern {s} = TreeDevice(uuid: \"{FLEX}\")\n");
        assert!(
            read(format!(
                "{ext}tiles vx_t\n\ttile 2()\nend\n{s}.tiles = vx_t\n"
            ))
            .is_ok()
        );
        // Not vislox's set, or a target not vislox's.
        assert!(
            read(format!(
                "{ext}tiles kueche\n\ttile 2()\nend\n{s}.tiles = kueche\n"
            ))
            .is_err()
        );
        let e = read(format!(
            "{ext}tiles vx_t\n\ttile 3(click: \"X(As;a)\", to: nord)\nend\n{s}.tiles = vx_t\n"
        ))
        .unwrap_err();
        assert!(e.contains("naming `nord`"), "{e}");
        // A target slug that is no extern of the file.
        let e = read(format!(
            "{ext}tiles vx_t\n\ttile 3(click: \"X(As;a)\", to: vx_nope)\nend\n{s}.tiles = vx_t\n"
        ))
        .unwrap_err();
        assert!(e.contains("names `vx_nope`"), "{e}");
        // A tile lox refuses.
        assert!(
            read(format!(
                "{ext}tiles vx_t\n\ttile 2(text: \"x\")\nend\n{s}.tiles = vx_t\n"
            ))
            .is_err()
        );
    }

    /// A push notification is drawn on a page as an `OutputRefLM` that
    /// mirrors the notification, with its message; the message is edited in
    /// its declaration, and a removal's `add_block` names the notification by
    /// UUID.
    #[test]
    fn a_push_notification_is_added_to_a_page_and_its_message_edited() {
        use crate::recipients_edit::fixture::*;
        let d = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = access_ctx(&d, &lock, &names, &claimed);
        let mut files = Files::default();
        let add = |settings: &[(&str, &str)], title: Option<&str>| Op::AddBlock {
            page: PAGE.into(),
            ty: "OutputRefLM".into(),
            title: title.map(String::from),
            params: BTreeMap::new(),
            settings: settings
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            wires: vec![],
            restores: None,
        };
        let a = apply_op(
            &mut files,
            &ctx,
            &add(
                &[
                    ("mirrors", REGEN),
                    ("On", "Es regnet"),
                    ("MinimumTime", "30min"),
                ],
                None,
            ),
        )
        .unwrap();
        assert_eq!(a.added.as_deref(), Some("vx_outputreflm_1"));
        let r = files.render();
        let page = r
            .values()
            .find(|t| t.contains("page \"Meldungen\""))
            .unwrap();
        let s = slug(REGEN);
        assert!(
            page.contains(&format!(
                "vx_outputreflm_1 = OutputRefLM(mirrors: {s}, On: \"Es regnet\", MinimumTime: 30min)"
            )),
            "{page}"
        );
        assert!(
            r["edits.loxl"].contains(&format!("extern {s} = Notification(uuid: \"{REGEN}\")")),
            "{}",
            r["edits.loxl"]
        );
        // Read back, the extern stays: the block names it.
        let back = Files::from_texts(&r).unwrap();
        assert_eq!(back.render(), r);
        let out = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        let u = lock.uuid_of("vx_outputreflm_1").unwrap().to_string();
        let el = find(&out.xml.root, &u).unwrap();
        assert_eq!(el.attr("Type"), Some("OutputRefLM"));
        assert!(el.attr("Ref").unwrap().eq_ignore_ascii_case(REGEN));
        let lm = el
            .child_elements()
            .find(|c| c.name == "LoggerMailer")
            .unwrap();
        assert_eq!(lm.attr("On"), Some("Es regnet"));
        assert_eq!(lm.attr("MinimumTime"), Some("1800000"));
        // Its message is edited in its declaration.
        let ctx2 = Ctx {
            doc: &out,
            lock: &lock,
            ..ctx
        };
        let set = |name: &str, v: Option<&str>| Op::Setting {
            uuid: u.clone(),
            name: name.into(),
            value: v.map(String::from),
        };
        let p = apply_op(&mut files, &ctx2, &set("Off", Some("Trocken"))).unwrap();
        assert_eq!(p.previous, serde_json::Value::Null);
        let p = apply_op(&mut files, &ctx2, &set("MinimumTime", Some("12h"))).unwrap();
        assert_eq!(p.previous, serde_json::json!("30min"));
        assert!(
            apply_op(&mut files, &ctx2, &set("MinimumTime", Some("bald")))
                .unwrap_err()
                .contains("a duration")
        );
        let out2 = compile_files(&d, &files, &mut l).unwrap();
        let lm = find(&out2.xml.root, &u)
            .unwrap()
            .child_elements()
            .find(|c| c.name == "LoggerMailer")
            .unwrap()
            .clone();
        assert_eq!(lm.attr("Off"), Some("Trocken"));
        assert_eq!(lm.attr("MinimumTime"), Some("43200000"));
        // Taken out again: the add_block that brings it back names the
        // notification by UUID.
        let ctx3 = Ctx { doc: &out2, ..ctx2 };
        let rm = apply_op(&mut files, &ctx3, &Op::RemoveBlock { uuid: u.clone() }).unwrap();
        let back: Op = serde_json::from_value(rm.previous).unwrap();
        let Op::AddBlock { settings, .. } = &back else {
            panic!("{back:?}")
        };
        assert_eq!(settings["mirrors"], REGEN);
        assert_eq!(settings["Off"], "Trocken");
        // The extern of the notification went with it.
        assert!(files.claims.0.is_empty(), "{:?}", files.claims.0);
        // A refused add changes nothing: no orphan extern of the notification.
        let mut f = Files::default();
        let before = f.clone();
        assert!(
            apply_op(
                &mut f,
                &ctx,
                &add(&[("mirrors", REGEN), ("MinimumTime", "bald")], None)
            )
            .unwrap_err()
            .contains("a duration")
        );
        assert_eq!(f, before);
        for bad in ["-5", "-5s", "5 s", "5d", ""] {
            assert!(duration(bad).is_none(), "{bad}");
        }
        assert_eq!(duration("90"), Some(Value::Number("90".into())));
        // An extern of the notification already there, in another case: one key.
        let mut f = Files::default();
        f.claims.0.insert(
            REGEN.to_ascii_uppercase(),
            Claim {
                ty: "Notification".into(),
                ..Default::default()
            },
        );
        apply_op(&mut f, &ctx, &add(&[("mirrors", REGEN)], None)).unwrap();
        assert_eq!(f.claims.0.len(), 1);
        assert_eq!(f.render()["edits.loxl"].matches("extern ").count(), 1);
        // One of another type is refused, not overwritten.
        let mut f = Files::default();
        f.claims.0.insert(
            REGEN.into(),
            Claim {
                ty: "DayTimer".into(),
                ..Default::default()
            },
        );
        let was = f.clone();
        assert!(
            apply_op(&mut f, &ctx, &add(&[("mirrors", REGEN)], None))
                .unwrap_err()
                .contains("not a push notification")
        );
        assert_eq!(f, was);
        // Refused: no notification, a name, another object, a constant.
        let err = |op: Op| apply_op(&mut Files::default(), &ctx, &op).unwrap_err();
        assert!(err(add(&[], None)).contains("which push notification"));
        assert!(err(add(&[("mirrors", REGEN)], Some("X"))).contains("notification's name"));
        assert!(err(add(&[("mirrors", ALICE)], None)).contains("no push notification"));
        assert!(err(add(&[("mirrors", REGEN), ("Sound", "1")], None)).contains("cannot be set"));
    }

    #[test]
    fn a_recipients_claim_vislox_did_not_write_is_refused() {
        use crate::recipients_edit::fixture::FROST;
        let s = slug(FROST);
        let ext = format!("extern {s} = Notification(uuid: \"{FROST}\")\n");
        let set = "recipients vx_r\n\tuser \"alice\"\nend\n";
        let read = |t: String| Files::from_texts(&BTreeMap::from([("edits.loxl".to_string(), t)]));
        let f = read(format!("{ext}{set}{s}.recipients = vx_r\n")).unwrap();
        assert_eq!(f.claims.6.claims[&s], "vx_r");
        assert!(
            read(format!(
                "{ext}recipients familie\n\tuser \"alice\"\nend\n{s}.recipients = familie\n"
            ))
            .is_err()
        );
        assert!(read(format!("{ext}{s}.recipients = vx_b\n")).is_err());
        assert!(
            read(format!("{set}{s}.recipients = vx_r\n"))
                .unwrap_err()
                .contains("neither an extern")
        );
    }

    // ── deleting and renaming managed blocks (#80) ──────────────────────

    /// `l` with the lockfile edits of an op made, as the edit writes them.
    fn edit_lock(l: &Lockfile, e: &crate::adopt::LockEdits) -> Lockfile {
        let bytes = serde_json::to_vec(l).unwrap();
        serde_json::from_slice(&crate::adopt::write_lock(&bytes, e).unwrap()).unwrap()
    }

    fn ctx_of<'a>(
        doc: &'a LoxoneDoc,
        base: &'a LoxoneDoc,
        lock: &'a Lock,
        deployed: Option<&'a LoxoneDoc>,
        names: &'a BTreeSet<String>,
    ) -> Ctx<'a> {
        static NONE_A: std::sync::LazyLock<BTreeMap<String, crate::adopt::Adoption>> =
            std::sync::LazyLock::new(BTreeMap::new);
        static NONE_P: std::sync::LazyLock<BTreeMap<String, Vec<String>>> =
            std::sync::LazyLock::new(BTreeMap::new);
        static NONE_W: std::sync::LazyLock<BTreeSet<Wire>> =
            std::sync::LazyLock::new(BTreeSet::new);
        static NONE_C: std::sync::LazyLock<crate::set_claim::Claimed> =
            std::sync::LazyLock::new(Default::default);
        Ctx {
            doc,
            base,
            lock,
            deployed,
            names,
            lets: &NO_LETS,
            drawn: &NONE_W,
            adoptions: &NONE_A,
            page_adoptions: &NONE_P,
            claimed: &NONE_C,
            templates: &NO_TEMPLATES,
            module: &crate::module_edit::NO_MODULE,
        }
    }

    /// The UUIDs of an element and its connectors, and the inputs they take.
    fn identity(doc: &LoxoneDoc, uuid: &str) -> String {
        let el = find(&doc.xml.root, uuid).unwrap();
        let mut out = format!("{} {:?}", uuid, el.attr("Title"));
        for c in el.child_elements() {
            out.push_str(&format!(
                " {}={}",
                c.attr("K").unwrap_or(""),
                c.attr("U").unwrap_or("")
            ));
        }
        out
    }

    #[test]
    fn a_deployed_block_is_deleted_and_comes_back_with_its_uuids() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        apply_op(
            &mut files,
            &ctx,
            &add(OG, "Monoflop", Some("Nachlauf"), &[("Time", "30")], &[]),
        )
        .unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let mono = l.objects["vx_monoflop_1"].uuid.clone();
        // A wire drawn here into it, from the Divide block.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        apply_op(
            &mut files,
            &ctx,
            &Op::Wire {
                from: [DIV.into(), "AQ".into()],
                to: [mono.clone(), "InputTrigger".into()],
            },
        )
        .unwrap();
        // Applied: the base, and so the deployed revision, has it now.
        let deployed = compile_files(&d, &files, &mut l).unwrap();
        let base = deployed.clone();
        let was = identity(&deployed, &mono);

        // Deleted: `removed`, the wire goes with it, and its undo is a restore.
        let lock = lock_of(&l);
        let ctx = ctx_of(&deployed, &base, &lock, Some(&deployed), &names);
        let r = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: mono.clone() }).unwrap();
        let again: Op = serde_json::from_value(r.previous).unwrap();
        let Op::RestoreBlock {
            ref source,
            ref wires,
            ref lock,
            ..
        } = again
        else {
            panic!("{again:?}")
        };
        assert_eq!(source, "vx_monoflop_1 = Monoflop(\"Nachlauf\", Time: 30)\n");
        assert_eq!(wires.len(), 1, "{wires:?}");
        assert_eq!(lock["uuid"], mono.as_str());
        let text = &files.render()["obergeschoss.loxl"];
        assert!(text.contains("removed vx_monoflop_1\n"), "{text}");
        assert!(!text.contains("= Monoflop"), "{text}");
        assert!(files.claims.1.is_empty());
        let gone = compile_files(&base, &files, &mut l).unwrap();
        assert!(find(&gone.xml.root, &mono).is_none());
        assert_eq!(lock_of(&l).tomb(&mono).map(|t| t.1), Some("vx_monoflop_1"));
        // The line is done once lox keeps the tombstone: the next edit drops it.
        let lock = lock_of(&l);
        assert!(!files.stale_lines(&lock).is_empty());
        files.tidy(&lock);
        assert!(!files.render().contains_key("obergeschoss.loxl"));

        let ctx = ctx_of(&gone, &base, &lock, Some(&deployed), &names);
        // A tampered or stale entry is refused, and changes nothing: another
        // object's connector as a port, another type, a field lox never writes,
        // another page.
        let div_port = find(&base.xml.root, DIV)
            .unwrap()
            .child_elements()
            .find_map(|c| c.attr("U"))
            .unwrap()
            .to_string();
        let tamper = |f: &dyn Fn(&mut serde_json::Value)| {
            let mut op = again.clone();
            if let Op::RestoreBlock { lock, .. } = &mut op {
                f(lock);
            }
            op
        };
        for (op, why) in [
            (
                tamper(&|l| l["ports"]["Q"] = serde_json::json!(div_port)),
                "not one of its connectors",
            ),
            (
                tamper(&|l| l["type"] = serde_json::json!("And")),
                "is not a Monoflop's",
            ),
            (
                tamper(&|l| l["owner"] = serde_json::json!("me")),
                "lox does not write",
            ),
            (
                tamper(&|l| l["page_uuid"] = serde_json::json!(EG)),
                "another page",
            ),
            (tamper(&|l| l["uuid"] = serde_json::json!(DIV)), "is not"),
        ] {
            let mut f = files.clone();
            let e = apply_op(&mut f, &ctx, &op).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
            assert_eq!(f, files);
        }
        // A wire whose other end is gone is left out and said; the block comes
        // back all the same.
        let mut lost = again.clone();
        if let Op::RestoreBlock { wires, .. } = &mut lost {
            wires.push(Ends {
                from: ["deadbeef-0000-0000-ffff504f94000001".into(), "AQ".into()],
                to: [String::new(), "InputTrigger".into()],
            });
        }
        let r = apply_op(&mut files.clone(), &ctx, &lost).unwrap();
        assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
        assert!(
            r.warnings[0].contains("not drawn again"),
            "{:?}",
            r.warnings
        );

        // Restored: the same UUIDs, its wire drawn again, the config as deployed.
        let r = apply_op(&mut files, &ctx, &again).unwrap();
        assert_eq!(
            serde_json::from_value::<Op>(r.previous).unwrap(),
            Op::RemoveBlock { uuid: mono.clone() }
        );
        assert!(files.lock_edits.untomb.contains(&mono));
        l = edit_lock(&l, &files.lock_edits);
        files.lock_edits = Default::default();
        let back = compile_files(&base, &files, &mut l).unwrap();
        assert_eq!(identity(&back, &mono), was);
        let diff = loxone_config::diff::diff(&deployed, &back);
        assert!(diff.logic_is_empty(), "{diff:?}");
        assert!(l.removed.is_empty(), "{:?}", l.removed);

        // A restore lox has no tombstone for is refused.
        let lock = lock_of(&l);
        let ctx = ctx_of(&back, &base, &lock, Some(&deployed), &names);
        let e = apply_op(&mut files, &ctx, &again).unwrap_err();
        assert!(e.contains("not deleted"), "{e}");
    }

    /// An AND gate Loxone Config drew, adopted here (#72) with a wire into it
    /// from the virtual input and two out of it, as `lox logic adopt` writes
    /// them in vislox's names.
    fn adopted_gate() -> (Files, Lockfile) {
        let vi = slug(VI);
        let meter = slug("10000005-0000-0050-ffff504f94000001");
        let licht = slug("10000009-0000-0090-ffff504f94000001");
        let texts = BTreeMap::from([
            (
                "erdgeschoss.loxl".to_string(),
                format!(
                    "page \"Erdgeschoss\"\n# Adopted from Loxone Config\nvx_gate = And(\"O1001\", \
                     In1: {vi})\n{meter}.Power <- vx_gate\n{licht}.In1 <- vx_gate\n"
                ),
            ),
            (
                CLAIMS.to_string(),
                format!(
                    "extern {vi} = VirtualIn(uuid: \"{VI}\")\nextern {meter} = MeterAbsUni(uuid: \
                     \"10000005-0000-0050-ffff504f94000001\")\nextern {licht} = Or(uuid: \
                     \"10000009-0000-0090-ffff504f94000001\")\n"
                ),
            ),
        ]);
        let mut f = Files::from_texts(&texts).unwrap();
        f.repin();
        let l: Lockfile = serde_json::from_value(serde_json::json!({
            "lockfile_version": 8,
            "objects": { "vx_gate": { "uuid": AND, "type": "And",
                "ports": { "I1": "10000004-0000-0041-00ff504f94000001", "I2": "10000004-0000-0042-01ff504f94000001", "Q": "10000004-0000-0043-02ff504f94000001" },
                "layout": { "px": 4032, "py": 768, "px2": 5376, "py2": 1464 },
                "page_uuid": EG } },
            "extern_wires": [
                { "from": "10000004-0000-0043-02ff504f94000001", "to": "10000005-0000-0051-00ff504f94000001" },
                { "from": "10000004-0000-0043-02ff504f94000001", "to": "10000009-0000-0091-00ff504f94000001" } ]
        }))
        .unwrap();
        (f, l)
    }

    #[test]
    fn an_adopted_block_is_deleted_with_its_wires_and_comes_back() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let (mut files, mut l) = adopted_gate();
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert!(loxone_config::diff::diff(&d, &out).logic_is_empty());
        let was = identity(&out, AND);

        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, Some(&d), &names);
        let r = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: AND.into() }).unwrap();
        let again: Op = serde_json::from_value(r.previous).unwrap();
        let Op::RestoreBlock {
            adopted,
            ref source,
            ref externs,
            ref lock_wires,
            ..
        } = again
        else {
            panic!("{again:?}")
        };
        assert!(adopted);
        assert_eq!(source.lines().count(), 3, "{source}");
        assert_eq!(externs.len(), 3, "{externs:?}");
        assert_eq!(lock_wires.len(), 2, "{lock_wires:?}");
        // Its externs go with it: nothing else names them.
        let r = files.render();
        assert!(!r[CLAIMS].contains("extern"), "{r:?}");
        assert!(r["erdgeschoss.loxl"].contains("removed vx_gate\n"), "{r:?}");
        let gone = compile_files(&d, &files, &mut l).unwrap();
        assert!(find(&gone.xml.root, AND).is_none());
        let diff = loxone_config::diff::diff(&d, &gone);
        assert_eq!(diff.removed.len(), 1, "{diff:?}");

        let lock = lock_of(&l);
        files.tidy(&lock);
        let ctx = ctx_of(&gone, &d, &lock, Some(&d), &names);
        apply_op(&mut files, &ctx, &again).unwrap();
        l = edit_lock(&l, &files.lock_edits);
        files.lock_edits = Default::default();
        let back = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(identity(&back, AND), was);
        let diff = loxone_config::diff::diff(&d, &back);
        assert!(diff.logic_is_empty(), "{diff:?}");
    }

    #[test]
    fn a_block_another_declaration_feeds_from_is_not_deleted() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let lock = Lock {
            slugs: BTreeMap::from([
                ("vx_a".to_string(), AND.to_string()),
                (
                    "vx_b".to_string(),
                    "10000005-0000-0050-ffff504f94000001".to_string(),
                ),
            ]),
            entries: BTreeMap::from([
                ("vx_a".to_string(), serde_json::json!({ "uuid": AND })),
                (
                    "vx_b".to_string(),
                    serde_json::json!({ "uuid": "10000005-0000-0050-ffff504f94000001" }),
                ),
            ]),
            ..Default::default()
        };
        let mut f = Files::from_texts(&BTreeMap::from([(
            "eg.loxl".to_string(),
            "page \"Erdgeschoss\"\n# Adopted from Loxone Config\nvx_a = And(\"O1001\")\nvx_b = \
             MeterAbsUni(\"Zähler\", Power: vx_a)\n"
                .to_string(),
        )]))
        .unwrap();
        let ctx = ctx_of(&d, &d, &lock, Some(&d), &names);
        let e = apply_op(&mut f, &ctx, &Op::RemoveBlock { uuid: AND.into() }).unwrap_err();
        assert!(e.contains("Zähler takes a wire from it"), "{e}");
        assert_eq!(delete_refusal(&f, &ctx, AND), Some(e));
        // The block it feeds goes first, then it.
        apply_op(
            &mut f,
            &ctx,
            &Op::RemoveBlock {
                uuid: "10000005-0000-0050-ffff504f94000001".into(),
            },
        )
        .unwrap();
        apply_op(&mut f, &ctx, &Op::RemoveBlock { uuid: AND.into() }).unwrap();
        let text = &f.render()["eg.loxl"];
        assert!(text.contains("removed vx_a\nremoved vx_b\n"), "{text}");
    }

    #[test]
    fn a_block_is_renamed_keeping_its_uuids_and_the_config() {
        let d = doc("house.Loxone");
        let names = BTreeSet::from(["flur".to_string()]);
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        apply_op(
            &mut files,
            &ctx,
            &add(OG, "Monoflop", None, &[("Time", "30")], &[]),
        )
        .unwrap();
        apply_op(&mut files, &ctx, &add(OG, "And", Some("Licht"), &[], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let (mono, and) = (
            l.objects["vx_monoflop_1"].uuid.clone(),
            l.objects["vx_and_1"].uuid.clone(),
        );
        // Wired, and the Monoflop among nothing else: a wire names it by its slug.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        apply_op(
            &mut files,
            &ctx,
            &Op::Wire {
                from: [mono.clone(), "Q".into()],
                to: [and.clone(), "I1".into()],
            },
        )
        .unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let was = (identity(&out, &mono), identity(&out, &and));

        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let rename = |slug: &str| Op::Rename {
            uuid: mono.clone(),
            slug: slug.into(),
        };
        // Names lox or vislox would refuse.
        for (bad, why) in [
            ("vx_monoflop_1", "already"),
            ("treppe", "named vx_"),
            ("vx_Treppe", "invalid slug"),
            ("vx_and_1", "taken"),
            ("flur", "named vx_"),
            ("vx_a__b", "expression"),
            ("vx_10000006_0000_0060_ffff504f94000001", "Loxone Config's"),
        ] {
            let e = apply_op(&mut files.clone(), &ctx, &rename(bad)).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
        let r = apply_op(&mut files, &ctx, &rename("vx_treppe")).unwrap();
        assert_eq!(
            serde_json::from_value::<Op>(r.previous).unwrap(),
            rename("vx_monoflop_1")
        );
        let all = files.render();
        let text = &all["obergeschoss.loxl"];
        // It keeps the label its slug gave it, and lox is told.
        assert!(
            text.contains("vx_treppe = Monoflop(\"vx_monoflop_1\", Time: 30)\n"),
            "{text}"
        );
        assert!(
            text.contains("moved vx_monoflop_1 -> vx_treppe\n"),
            "{text}"
        );
        assert!(
            all[CLAIMS].contains("vx_and_1.In1 <- vx_treppe.Q"),
            "{all:?}"
        );
        let renamed = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(l.objects["vx_treppe"].uuid, mono);
        assert!(!l.objects.contains_key("vx_monoflop_1"));
        assert_eq!((identity(&renamed, &mono), identity(&renamed, &and)), was);
        let diff = loxone_config::diff::diff(&out, &renamed);
        assert!(diff.logic_is_empty(), "{diff:?}");

        // Undone: the slug again, and the label shown by it.
        let lock = lock_of(&l);
        assert!(!files.stale_lines(&lock).is_empty());
        files.tidy(&lock);
        let ctx = ctx_of(&renamed, &d, &lock, None, &names);
        apply_op(&mut files, &ctx, &rename("vx_monoflop_1")).unwrap();
        let text = &files.render()["obergeschoss.loxl"];
        assert!(
            text.contains("vx_monoflop_1 = Monoflop(Time: 30)\n"),
            "{text}"
        );
        assert!(
            text.contains("moved vx_treppe -> vx_monoflop_1\n"),
            "{text}"
        );
        assert!(!text.contains("moved vx_monoflop_1 ->"), "{text}");
        let back = compile_files(&d, &files, &mut l).unwrap();
        assert!(loxone_config::diff::diff(&out, &back).logic_is_empty());

        // A module's own block is renamed in its own file.
        let e = apply_op(
            &mut files.clone(),
            &ctx_of(
                &back,
                &d,
                &Lock {
                    slugs: BTreeMap::from([("flur".to_string(), DIV.to_string())]),
                    ..Default::default()
                },
                None,
                &names,
            ),
            &Op::Rename {
                uuid: DIV.into(),
                slug: "vx_x".into(),
            },
        )
        .unwrap_err();
        assert!(e.contains("lox logic rename"), "{e}");
    }

    #[test]
    fn a_renamed_adopted_block_keeps_its_wires_and_program() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let (mut files, mut l) = adopted_gate();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, Some(&d), &names);
        apply_op(
            &mut files,
            &ctx,
            &Op::Rename {
                uuid: AND.into(),
                slug: "vx_tor".into(),
            },
        )
        .unwrap();
        let text = &files.render()["erdgeschoss.loxl"];
        assert!(text.contains("vx_tor = And(\"O1001\""), "{text}");
        assert!(text.contains(".Power <- vx_tor\n"), "{text}");
        let renamed = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(l.objects["vx_tor"].uuid, AND);
        let diff = loxone_config::diff::diff(&d, &renamed);
        assert!(diff.logic_is_empty(), "{diff:?}");

        // A program block's file follows its slug.
        let lock = lock_of(&l);
        files.tidy(&lock);
        let ctx = ctx_of(&renamed, &d, &lock, Some(&d), &names);
        apply_op(
            &mut files,
            &ctx,
            &add(
                EG,
                "Code16",
                Some("Hue"),
                &[],
                &[("Code", "setoutput(0, 1);\n")],
            ),
        )
        .unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let hue = l.objects["vx_code16_1"].uuid.clone();
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, Some(&d), &names);
        apply_op(
            &mut files,
            &ctx,
            &Op::Rename {
                uuid: hue.clone(),
                slug: "vx_hue".into(),
            },
        )
        .unwrap();
        assert_eq!(
            files.programs().into_iter().collect::<Vec<_>>(),
            vec![("vx_hue.c", "setoutput(0, 1);\n")]
        );
        assert!(
            files
                .render()
                .values()
                .any(|t| t.contains("vx_hue = Code16(\"Hue\", Code: file(\"vx_hue.c\"))")),
            "{:?}",
            files.render()
        );
        let again = compile_files(&d, &files, &mut l).unwrap();
        assert!(loxone_config::diff::diff(&out, &again).logic_is_empty());
        assert_eq!(l.objects["vx_hue"].uuid, hue);
    }

    #[test]
    fn a_wire_whose_end_cannot_be_named_refuses_the_delete() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        // `vx_c` is declared but never compiled: the lock names no UUID for it,
        // so an undo could not draw the wire to it again.
        let lock = Lock {
            slugs: BTreeMap::from([("vx_a".to_string(), AND.to_string())]),
            entries: BTreeMap::from([("vx_a".to_string(), serde_json::json!({ "uuid": AND }))]),
            ..Default::default()
        };
        let f = Files::from_texts(&BTreeMap::from([
            (
                "eg.loxl".to_string(),
                "page \"Erdgeschoss\"\n# Adopted from Loxone Config\nvx_a = And(\"O1001\")\nvx_c = \
                 And(\"C\")\n"
                    .to_string(),
            ),
            (CLAIMS.to_string(), "vx_c.I1 <- vx_a.Q\n".to_string()),
        ]))
        .unwrap();
        let ctx = ctx_of(&d, &d, &lock, Some(&d), &names);
        let e = delete_refusal(&f, &ctx, AND).unwrap();
        assert!(e.contains("cannot find"), "{e}");
        let mut g = f.clone();
        assert_eq!(
            apply_op(&mut g, &ctx, &Op::RemoveBlock { uuid: AND.into() }).unwrap_err(),
            e
        );
        assert_eq!(g, f);
    }

    // ── constants (#81) ─────────────────────────────────────────────────

    fn let_op(name: &str, v: Option<&str>) -> Op {
        Op::Let {
            name: name.into(),
            value: v.map(Into::into),
            source: None,
        }
    }

    /// The constant of connector `key` of `uuid` in `doc`.
    fn def_of(doc: &LoxoneDoc, uuid: &str, key: &str) -> Option<String> {
        current(doc, &param(uuid, key, None))
    }

    #[test]
    fn a_constant_made_here_is_bound_to_ports_and_changes_them_all() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        apply_op(
            &mut files,
            &ctx,
            &add(OG, "Monoflop", Some("Nachlauf"), &[("Time", "30")], &[]),
        )
        .unwrap();
        // Made, then bound in one request: the Divide block's divisor (a
        // claim on an extern of Loxone Config's).
        let made = apply_op(&mut files, &ctx, &let_op("vx_zeit", Some("100"))).unwrap();
        assert_eq!(made.previous, serde_json::json!(null));
        let bound = apply_op(&mut files, &ctx, &param(DIV, "Input2", Some("vx_zeit"))).unwrap();
        assert_eq!(bound.previous, serde_json::json!(null));
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(def_of(&out, DIV, "Input2").as_deref(), Some("100"));
        // And the Monoflop added here: its declaration names the constant.
        let mono = l.objects["vx_monoflop_1"].uuid.clone();
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let was = apply_op(&mut files, &ctx, &param(&mono, "Time", Some("vx_zeit"))).unwrap();
        assert_eq!(was.previous, serde_json::json!("30"));
        let text = files.render();
        assert!(
            text["edits.loxl"].contains("let vx_zeit = 100\n"),
            "{}",
            text["edits.loxl"]
        );
        assert!(
            text["edits.loxl"].contains(".In2 = vx_zeit\n"),
            "{}",
            text["edits.loxl"]
        );
        assert!(
            text["obergeschoss.loxl"].contains("Time: vx_zeit"),
            "{}",
            text["obergeschoss.loxl"]
        );
        // Read back as written.
        assert_eq!(Files::from_texts(&text).unwrap().render(), text);
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(def_of(&out, &mono, "Time").as_deref(), Some("100"));
        // Changing the constant changes both ports.
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let r = apply_op(&mut files, &ctx, &let_op("vx_zeit", Some("250"))).unwrap();
        assert_eq!(r.previous, serde_json::json!("100"));
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(def_of(&out, DIV, "Input2").as_deref(), Some("250"));
        assert_eq!(def_of(&out, &mono, "Time").as_deref(), Some("250"));
        // A constant still bound is not dropped.
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let e = apply_op(&mut files.clone(), &ctx, &let_op("vx_zeit", None)).unwrap_err();
        assert!(e.contains("still bound to 2 ports"), "{e}");
        // The undo of making it and binding it: the bindings go first, then
        // the constant.
        let ops = [
            let_op("vx_neu", Some("5")),
            param(DIV, "Input2", Some("vx_neu")),
        ];
        let mut prev = Vec::new();
        for op in &ops {
            prev.push(apply_op(&mut files, &ctx, op).unwrap().previous);
        }
        let undo = undo_ops(&ops, &prev, &[None, None]).unwrap();
        assert_eq!(
            undo,
            vec![
                param(DIV, "Input2", Some("vx_zeit")),
                let_op("vx_neu", None)
            ]
        );
        for op in &undo {
            apply_op(&mut files, &ctx, op).unwrap();
        }
        assert!(!files.claims.8.contains_key("vx_neu"));
        assert_eq!(files.render(), text_with(&files));
    }

    /// [`compile_files`] with the module's own files too (`own`: path in
    /// the module → text) and the rewrites of `files.module` over them.
    fn compile_own(
        base: &LoxoneDoc,
        own: &BTreeMap<String, String>,
        files: &Files,
        lock: &mut Lockfile,
    ) -> Result<LoxoneDoc, String> {
        let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let module = tmp.path().join("module");
        let dir = module.join(DIR);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for (name, t) in files.all_texts() {
            std::fs::write(dir.join(name), t).map_err(|e| e.to_string())?;
        }
        for (path, t) in own {
            let t = files.module.get(&format!("module/{path}")).unwrap_or(t);
            std::fs::write(module.join(path), t).map_err(|e| e.to_string())?;
        }
        let m =
            loxone_config::cli::read_module(module.to_str().unwrap()).map_err(|e| e.to_string())?;
        let opts = CompileOptions {
            machine: [0x50, 0x4f, 0x94, 0, 0, 1],
            mint_time_unix: 0,
            page_title: None,
            allow_removals: false,
            accept_version: None,
            relayout: vec![],
        };
        let out = logic::compile(base, &m, lock, &opts).map_err(|e| e.to_string())?;
        LoxoneDoc::parse(&out.to_bytes()).map_err(|e| e.to_string())
    }

    fn defaults_op(
        ty: &str,
        port: &str,
        v: Option<&str>,
        source: Option<&str>,
        blocks: Option<&[&str]>,
    ) -> Op {
        Op::Defaults {
            ty: ty.into(),
            port: port.into(),
            value: v.map(Into::into),
            source: source.map(Into::into),
            blocks: blocks.map(|b| b.iter().map(|s| s.to_string()).collect()),
        }
    }

    /// The module's own `let` and `defaults` values are rewritten where they
    /// are (#81): one value, the rest of the file byte for byte, compiled by
    /// lox, refused on a stale line, and undone to the exact old text.
    #[test]
    fn a_module_constant_and_default_are_rewritten_in_place_and_undone() {
        let d = doc("house.Loxone");
        let own_text = "page \"Obergeschoss\"\n\n# Werte des Hauses\nlet nachlauf  =  30min   # bis aus\nlet name = \"Haus\"\ndefaults Monoflop(\n\tTime: nachlauf, # alle\n\tReset: 0,\n)\n\nlicht = Monoflop(\"Licht\")\nflur = Monoflop(\"Flur\", Time: 10s)\nteil = Div(\"Teil\")\n";
        let own: BTreeMap<String, String> =
            [("haus.loxl".to_string(), own_text.to_string())].into();
        let mut l = Lockfile::default();
        let out = compile_own(&d, &own, &Files::default(), &mut l).unwrap();
        let licht = l.objects["licht"].uuid.clone();
        let flur = l.objects["flur"].uuid.clone();
        let teil = l.objects["teil"].uuid.clone();
        let before_time = def_of(&out, &licht, "Time").unwrap();
        let lock = lock_of(&l);
        let names: BTreeSet<String> = ["nachlauf", "name", "licht", "flur", "teil"]
            .into_iter()
            .map(String::from)
            .collect();
        let lets: BTreeSet<String> = ["nachlauf".to_string(), "name".to_string()].into();
        let src = crate::module_edit::ModuleSrc {
            dir: "module/vislox".into(),
            texts: [("module/haus.loxl".to_string(), own_text.to_string())].into(),
        };
        let mut ctx = ctx_of(&out, &d, &lock, None, &names);
        ctx.lets = &lets;
        ctx.module = &src;
        let mut files = Files::default();
        let line = "let nachlauf  =  30min   # bis aus";
        let mlet = |v: Option<&str>, source: Option<&str>| Op::Let {
            name: "nachlauf".into(),
            value: v.map(Into::into),
            source: source.map(Into::into),
        };
        // Refused, changing nothing: no line, a stale line, no value, a text.
        for (op, why) in [
            (mlet(Some("45min"), None), "`source`"),
            (
                mlet(Some("45min"), Some("let nachlauf = 30min")),
                "changed since",
            ),
            (mlet(None, Some(line)), "never drops"),
            (mlet(Some("hoch"), Some(line)), "no constant"),
            (
                Op::Let {
                    name: "name".into(),
                    value: Some("1".into()),
                    source: Some("let name = \"Haus\"".into()),
                },
                "text constant",
            ),
        ] {
            let e = apply_op(&mut files, &ctx, &op).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
            assert_eq!(files, Files::default());
        }
        // Changed: that value and nothing else.
        let r = apply_op(&mut files, &ctx, &mlet(Some("45min"), Some(line))).unwrap();
        let new_text = own_text.replace("=  30min   #", "=  45min   #");
        assert_eq!(files.module["module/haus.loxl"], new_text);
        assert!(files.render().is_empty(), "{:?}", files.render());
        assert_eq!(
            r.previous,
            serde_json::json!({ "op": "let", "name": "nachlauf", "value": "30min",
                                "source": "let nachlauf  =  45min   # bis aus" })
        );
        let out2 = compile_own(&d, &own, &files, &mut l).unwrap();
        let after_time = def_of(&out2, &licht, "Time").unwrap();
        let (b, a): (f64, f64) = (before_time.parse().unwrap(), after_time.parse().unwrap());
        assert_eq!(a / b, 1.5, "{before_time} → {after_time}");
        assert_eq!(def_of(&out2, &flur, "Time"), def_of(&out, &flur, "Time"));
        // Undone through `undo`: the exact old text, byte for byte.
        let ops = [mlet(Some("45min"), Some(line))];
        let undo = undo_ops(&ops, std::slice::from_ref(&r.previous), &[None]).unwrap();
        let mut back = files.clone();
        apply_op(&mut back, &ctx, &undo[0]).unwrap();
        assert_eq!(back.module["module/haus.loxl"], own_text);

        // The default: its consent names exactly the blocks it changes, the
        // module's own `licht` (`flur` binds Time itself).
        let stmt = "defaults Monoflop(\n\tTime: nachlauf, # alle\n\tReset: 0,\n)";
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &defaults_op("Monoflop", "Time", Some("5min"), Some(stmt), None),
        )
        .unwrap_err();
        assert!(e.contains("changes 1 block: licht"), "{e}");
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &defaults_op(
                "Monoflop",
                "Time",
                Some("5min"),
                Some(stmt),
                Some(&["licht", "flur"]),
            ),
        )
        .unwrap_err();
        assert!(e.contains("Confirm exactly these"), "{e}");
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &defaults_op(
                "Monoflop",
                "Time",
                Some("5min"),
                Some("defaults Monoflop(Time: 1)"),
                Some(&["licht"]),
            ),
        )
        .unwrap_err();
        assert!(e.contains("changed since"), "{e}");
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &defaults_op(
                "Monoflop",
                "Trigger",
                Some("1"),
                Some(stmt),
                Some(&["licht", "flur"]),
            ),
        )
        .unwrap_err();
        assert!(e.contains("never adds one"), "{e}");
        let before = files.clone();
        let r = apply_op(
            &mut files,
            &ctx,
            &defaults_op(
                "Monoflop",
                "Time",
                Some("5min"),
                Some(stmt),
                Some(&["licht"]),
            ),
        )
        .unwrap();
        let new2 = new_text.replace("\tTime: nachlauf, # alle", "\tTime: 5min, # alle");
        assert_eq!(files.module["module/haus.loxl"], new2);
        let out3 = compile_own(&d, &own, &files, &mut l).unwrap();
        assert_eq!(
            def_of(&out3, &licht, "Time").map(|t| t.parse::<f64>().unwrap() / b),
            Some(1.0 / 6.0)
        );
        let ops = [defaults_op(
            "Monoflop",
            "Time",
            Some("5min"),
            Some(stmt),
            Some(&["licht"]),
        )];
        let undo = undo_ops(&ops, std::slice::from_ref(&r.previous), &[None]).unwrap();
        let mut back = files.clone();
        apply_op(&mut back, &ctx, &undo[0]).unwrap();
        assert_eq!(back, before);

        // A default made here: in vislox's file of claims, reaching the
        // module's own block; undone, it is gone.
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &defaults_op("Div", "Input2", Some("4"), None, Some(&[])),
        )
        .unwrap_err();
        assert!(e.contains("changes 1 block: teil"), "{e}");
        let before = files.clone();
        let r = apply_op(
            &mut files,
            &ctx,
            &defaults_op("Div", "Input2", Some("4"), None, Some(&["teil"])),
        )
        .unwrap();
        assert_eq!(r.previous["value"], serde_json::Value::Null);
        assert!(
            files.render()["edits.loxl"].contains("\ndefaults Div(In2: 4)\n"),
            "{}",
            files.render()["edits.loxl"]
        );
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().claims,
            files.claims
        );
        let out4 = compile_own(&d, &own, &files, &mut l).unwrap();
        assert_eq!(def_of(&out4, &teil, "Input2").as_deref(), Some("4"));
        let undo = undo_ops(
            &[defaults_op(
                "Div",
                "Input2",
                Some("4"),
                None,
                Some(&["teil"]),
            )],
            &[r.previous],
            &[None],
        )
        .unwrap();
        apply_op(&mut files, &ctx, &undo[0]).unwrap();
        assert_eq!(files, before);
        assert!(files.render().is_empty());
    }

    /// The files' texts, read back and rendered again: a fixed point.
    fn text_with(files: &Files) -> BTreeMap<String, String> {
        Files::from_texts(&files.render()).unwrap().render()
    }

    #[test]
    fn a_constant_takes_a_name_of_its_own_and_a_value_lox_takes() {
        let d = doc("house.Loxone");
        let names: BTreeSet<String> = ["vx_belegt".to_string(), "schwelle".to_string()].into();
        let lets: BTreeSet<String> = ["schwelle".to_string()].into();
        let lock = Lock::default();
        let mut ctx = ctx_of(&d, &d, &lock, None, &names);
        ctx.lets = &lets;
        let mut files = Files::default();
        let err = |files: &mut Files, op: Op| apply_op(files, &ctx, &op).unwrap_err();
        assert!(err(&mut files, let_op("zeit", Some("1"))).contains("vx_"));
        assert!(err(&mut files, let_op("vx_Zeit", Some("1"))).contains("invalid slug"));
        assert!(err(&mut files, let_op("vx_belegt", Some("1"))).contains("is taken"));
        assert!(err(&mut files, let_op(&slug(DIV), Some("1"))).contains("vx_"));
        // The module's own constant is rewritten in its file, which names the
        // line it changes (see `a_module_constant_and_default_are_rewritten_…`).
        assert!(err(&mut files, let_op("schwelle", Some("1"))).contains("module's own files"));
        assert!(err(&mut files, let_op("vx_x", None)).contains("has no constant"));
        assert!(err(&mut files, let_op("vx_x", Some("12,5"))).contains("no constant"));
        assert!(err(&mut files, let_op("vx_x", Some("\"Text\""))).contains("no constant"));
        for (v, written) in [
            ("-2.5", "-2.5"),
            ("30min", "30min"),
            ("70%", "70%"),
            ("19:00", "19:00"),
            ("on", "on"),
            ("off", "off"),
        ] {
            apply_op(&mut files, &ctx, &let_op("vx_x", Some(v))).unwrap();
            let t = files.render();
            assert!(
                t["edits.loxl"].contains(&format!("let vx_x = {written}\n")),
                "{t:?}"
            );
            assert_eq!(text_with(&files), t);
        }
        // A port may name the module's constant, or vislox's.
        let c = apply_op(&mut files, &ctx, &param(DIV, "Input2", Some("schwelle"))).unwrap();
        assert_eq!(c.previous, serde_json::json!(null));
        assert_eq!(
            files.claims.0[&DIV.to_ascii_lowercase()].sets["In2"],
            Value::Ref("schwelle".into())
        );
        // A name that is no constant is still no number.
        assert!(err(&mut files, param(DIV, "Input2", Some("vx_nichts"))).contains("not a number"));
    }

    #[test]
    fn the_claims_file_takes_only_constants_vislox_writes() {
        for (text, ok) in [
            ("let vx_a = 1\n", true),
            ("let vx_a = 2h\n", true),
            ("let a = 1\n", false),
            ("let vx_a = \"Text\"\n", false),
            ("let vx_10000006_0000_0060_ffff504f94000001 = 1\n", false),
        ] {
            assert_eq!(Claims::parse(text).is_ok(), ok, "{text}");
        }
        assert!(Claims::parse("extern x = Div(uuid: \"u\")\nlet vx_a = x.AQ\n").is_err());
    }

    #[test]
    fn a_bare_name_of_a_constant_on_an_adopted_block_is_a_value() {
        let texts: BTreeMap<String, String> = [
            ("edits.loxl".to_string(), "let vx_zeit = 30\n".to_string()),
            (
                "obergeschoss.loxl".to_string(),
                format!(
                    "page \"Obergeschoss\"\n\n# {}: drawn there.\nvx_nachlauf = Monoflop(\"Nachlauf\", Time: vx_zeit)\n",
                    crate::blocks::ADOPTED_LINE
                ),
            ),
        ]
        .into();
        let f = Files::from_texts(&texts).unwrap();
        let b = f.block("vx_nachlauf").unwrap().1;
        assert!(b.wires.is_empty(), "{b:?}");
        assert_eq!(b.get("Time"), Some(&Value::Ref("vx_zeit".into())));
        assert_eq!(constant_uses(&f, "vx_zeit"), 1);
    }

    #[test]
    fn a_constant_dropped_and_made_again_in_one_request_is_vislox_s_to_name() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let lock = Lock::default();
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let mut files = Files::default();
        apply_op(&mut files, &ctx, &let_op("vx_a", Some("1"))).unwrap();
        // The module's names leave vislox's own files out: what vislox's files
        // name is the request's, as it goes.
        let mut again = files.clone();
        for op in [let_op("vx_a", None), let_op("vx_a", Some("2"))] {
            apply_op(&mut again, &ctx, &op).unwrap();
        }
        assert_eq!(again.claims.8["vx_a"], Value::Number("2".into()));
        // Dropped earlier in the request, it names nothing.
        let mut gone = files.clone();
        apply_op(&mut gone, &ctx, &let_op("vx_a", None)).unwrap();
        let e = apply_op(&mut gone, &ctx, &param(DIV, "Input2", Some("vx_a"))).unwrap_err();
        assert!(e.contains("not a number"), "{e}");
        // vislox's own names count its constants.
        assert!(files.names("vx_a"));
    }

    #[test]
    fn the_module_s_names_leave_vislox_s_files_out_and_say_what_does_not_read() {
        let tmp = tempfile::tempdir().unwrap();
        let module = tmp.path().join("pages");
        std::fs::create_dir_all(module.join(DIR)).unwrap();
        std::fs::write(
            module.join("main.loxl"),
            "let t = 4\nlet aussen = licht.Q\nlicht = Monoflop(\"Licht\")\n",
        )
        .unwrap();
        std::fs::write(module.join(DIR).join("edits.loxl"), "let vx_a = 1\n").unwrap();
        let d = module_decls(&module).unwrap();
        assert_eq!(
            d.names,
            ["aussen", "licht", "t"]
                .map(String::from)
                .into_iter()
                .collect()
        );
        assert_eq!(d.lets, ["t".to_string()].into());
        assert_eq!(d.aliases, ["aussen".to_string()].into());
        std::fs::write(module.join("kaputt.loxl"), "let = (\n").unwrap();
        let e = module_decls(&module).unwrap_err();
        assert!(e.starts_with("kaputt.loxl: "), "{e}");
    }

    // ── template instances (#83) ────────────────────────────────────────

    fn inst(slug: &str, template: &str, args: &[(&str, &str)], label: Option<&str>) -> Op {
        Op::Instance {
            slug: slug.into(),
            page: OG.into(),
            value: Some(crate::template_edit::Call {
                template: template.into(),
                label: label.map(Into::into),
                args: args
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            }),
        }
    }

    #[test]
    fn a_library_template_instance_is_made_changed_and_dropped() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let vi = slug(VI);
        let op = inst(
            "vx_treppe",
            "Treppenlicht",
            &[("taster", &vi), ("dauer", "5min"), ("titel", "Flur")],
            None,
        );
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        // Undone, it is dropped.
        let undo = undo_ops(std::slice::from_ref(&op), &[a.previous], &[]).unwrap();
        assert_eq!(
            undo[0],
            Op::Instance {
                slug: "vx_treppe".into(),
                page: OG.into(),
                value: None
            }
        );
        let texts = files.render();
        let page = texts.values().find(|t| t.contains("vx_treppe")).unwrap();
        assert!(
            page.contains("# Template instances")
                && page.contains(&format!(
                    "vx_treppe = Treppenlicht(taster: {vi}, dauer: 5min, titel: \"Flur\")\n"
                )),
            "{page}"
        );
        // The library template is copied in, and the object's extern made.
        assert!(
            texts[TEMPLATES].contains("template Treppenlicht("),
            "{}",
            texts[TEMPLATES]
        );
        assert!(texts[CLAIMS].contains(&format!("extern {vi} = VirtualIn(uuid: \"{VI}\")")));
        let back = Files::from_texts(&texts).unwrap();
        assert_eq!(
            (&back.pages, &back.templates),
            (&files.pages, &files.templates)
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        let mono = l.objects["vx_treppe_ausgang"].uuid.clone();
        let el = find(&out.xml.root, &mono).unwrap();
        assert_eq!(el.attr_decoded("Title").as_deref(), Some("Flur"));
        // A member's value op is not taken for one below a Tree device created
        // here (#85), whether the lock knows it yet or not.
        for lk in [lock_of(&l), Lock::default()] {
            let ctx = ctx_of(&out, &d, &lk, None, &names);
            let title = Op::Title {
                uuid: mono.clone(),
                value: Some("Gang".into()),
            };
            let r = apply_op(&mut files.clone(), &ctx, &title);
            assert!(
                !r.as_ref().is_err_and(|e| e.contains("base config")),
                "{r:?}"
            );
        }
        // Its call as the op takes it: the text unquoted.
        let (_, def) = files.instance("vx_treppe").unwrap();
        let call = crate::template_edit::call_of(def);
        // As written: a quoted text stays one.
        assert_eq!(call.args["titel"], "\"Flur\"");
        assert_eq!(call.args["dauer"], "5min");
        // Changed: the same blocks.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let a = apply_op(
            &mut files,
            &ctx,
            &inst(
                "vx_treppe",
                "Treppenlicht",
                &[("taster", &vi), ("dauer", "2min")],
                None,
            ),
        )
        .unwrap();
        // What was, as written: the text quoted.
        assert_eq!(
            serde_json::from_value::<Op>(a.previous).unwrap(),
            inst(
                "vx_treppe",
                "Treppenlicht",
                &[("taster", &vi), ("dauer", "5min"), ("titel", "\"Flur\"")],
                None
            )
        );
        let out2 = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(l.objects["vx_treppe_ausgang"].uuid, mono);
        let el = find(&out2.xml.root, &mono).unwrap();
        assert_eq!(el.attr_decoded("Title").as_deref(), Some("Treppenlicht"));
        // Checked as lox checks a call.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out2, &d, &lock, None, &names);
        for (args, label, why) in [
            (
                vec![("taster", vi.as_str()), ("dauer", "5")],
                None,
                "duration parameter",
            ),
            (
                vec![("taster", vi.as_str()), ("nope", "1")],
                None,
                "no parameter `nope`",
            ),
            (vec![("dauer", "5min")], None, "needs taster"),
            (vec![("taster", vi.as_str())], Some("X"), "takes no label"),
            (vec![("taster", "flur")], None, "names nothing"),
            (vec![("taster", "vx_treppe")], None, "does not read itself"),
        ] {
            let e = apply_op(
                &mut files.clone(),
                &ctx,
                &inst("vx_treppe", "Treppenlicht", &args, label),
            )
            .unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
        }
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &inst("vx_treppe", "Bewegungslicht", &[("bewegung", &vi)], None),
        )
        .unwrap_err();
        assert!(e.contains("lox keeps its blocks' types"), "{e}");
        let e = apply_op(&mut files.clone(), &ctx, &inst("vx_x", "Nope", &[], None)).unwrap_err();
        assert!(e.contains("no template"), "{e}");
        // A name another one's blocks start with, or one starting with an instance's.
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &inst(
                "vx_treppe_ausgang",
                "Treppenlicht",
                &[("taster", &vi)],
                None,
            ),
        )
        .unwrap_err();
        assert!(e.contains("taken"), "{e}");
        let taken: BTreeSet<String> = ["vx_q_licht".to_string()].into();
        let ctx2 = ctx_of(&out2, &d, &lock, None, &taken);
        let e = apply_op(
            &mut files.clone(),
            &ctx2,
            &inst("vx_q", "Treppenlicht", &[("taster", &vi)], None),
        )
        .unwrap_err();
        assert!(e.contains("starts so"), "{e}");
        // Dropping: refused once deployed; else `removed` for each block.
        let drop = Op::Instance {
            slug: "vx_treppe".into(),
            page: OG.into(),
            value: None,
        };
        let ctx = ctx_of(&out2, &d, &lock, Some(&out2), &names);
        let e = apply_op(&mut files.clone(), &ctx, &drop).unwrap_err();
        assert!(e.contains("deployed"), "{e}");
        let ctx = ctx_of(&out2, &d, &lock, Some(&d), &names);
        let a = apply_op(&mut files, &ctx, &drop).unwrap();
        let texts = files.render();
        assert!(
            !texts.contains_key(TEMPLATES),
            "the copy goes with the last instance"
        );
        assert!(
            texts
                .values()
                .any(|t| t.contains("removed vx_treppe_ausgang"))
        );
        files = Files::from_texts(&texts).unwrap();
        assert!(
            files
                .pages
                .values()
                .any(|p| p.unstamped.contains("vx_treppe_ausgang"))
        );
        compile_files(&d, &files, &mut l).unwrap();
        assert!(!l.objects.contains_key("vx_treppe_ausgang"));
        // Made again before lox forgot it: the line is taken back.
        let lock = lock_of(&l);
        let _ = lock;
        let mut again = files.clone();
        apply_op(
            &mut again,
            &ctx,
            &serde_json::from_value(a.previous).unwrap(),
        )
        .unwrap();
        assert!(
            !again
                .render()
                .values()
                .any(|t| t.contains("removed vx_treppe"))
        );
    }

    #[test]
    fn an_argument_keeps_whether_it_was_a_text_or_a_constant_s_name() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let lets: BTreeSet<String> = ["hell".to_string()].into();
        let mut files = Files::default();
        let lock = Lock::default();
        let mut ctx = ctx_of(&d, &d, &lock, None, &names);
        ctx.lets = &lets;
        let vi = slug(VI);
        for (typed, written) in [
            ("\"hell\"", "\"hell\""),
            ("hell", "hell"),
            ("Flur oben", "\"Flur oben\""),
        ] {
            let mut f = files.clone();
            apply_op(
                &mut f,
                &ctx,
                &inst(
                    "vx_t",
                    "Treppenlicht",
                    &[("taster", &vi), ("titel", typed)],
                    None,
                ),
            )
            .unwrap();
            let (_, def) = f.instance("vx_t").unwrap();
            assert_eq!(
                def.args[1],
                ("titel".to_string(), written.to_string()),
                "{typed}"
            );
            // Saved again as Properties shows it, and through `previous` (Revert, undo): unchanged.
            let call = crate::template_edit::call_of(def);
            let again = Op::Instance {
                slug: "vx_t".into(),
                page: OG.into(),
                value: Some(call),
            };
            let a = apply_op(&mut f, &ctx, &again).unwrap();
            assert_eq!(f.instance("vx_t").unwrap().1.args[1].1, written);
            apply_op(&mut f, &ctx, &serde_json::from_value(a.previous).unwrap()).unwrap();
            assert_eq!(f.instance("vx_t").unwrap().1.args[1].1, written);
        }
        apply_op(
            &mut files,
            &ctx,
            &inst("vx_t", "Treppenlicht", &[("taster", &vi)], None),
        )
        .unwrap();
        // Another instance's blocks: by name, never the bare instance.
        let bad = [
            ("vx_t", "is an instance, no object: name one of its blocks"),
            ("vx_t.nope", "has no block `nope`; its blocks: ausgang"),
        ];
        for (arg, why) in bad {
            let e = apply_op(
                &mut files.clone(),
                &ctx,
                &inst("vx_u", "Treppenlicht", &[("taster", arg)], None),
            )
            .unwrap_err();
            assert!(e.contains(why), "{arg}: {e}");
        }
        apply_op(
            &mut files.clone(),
            &ctx,
            &inst("vx_u", "Treppenlicht", &[("taster", "vx_t.ausgang")], None),
        )
        .unwrap();
    }

    #[test]
    fn a_page_file_called_templates_stays_a_page_file() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let texts: BTreeMap<String, String> = [(
            TEMPLATES.to_string(),
            "page \"Templates\"\n\nvx_and_1 = And()\n".to_string(),
        )]
        .into();
        let mut files = Files::from_texts(&texts).unwrap();
        assert!(files.pages.contains_key(TEMPLATES) && files.templates.is_empty());
        assert_eq!(files.render()[TEMPLATES], files.pages[TEMPLATES].render());
        let lock = Lock::default();
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let e = apply_op(
            &mut files,
            &ctx,
            &inst("vx_t", "Treppenlicht", &[("taster", &slug(VI))], None),
        )
        .unwrap_err();
        assert!(e.contains("which is a page's file here"), "{e}");
    }

    #[test]
    fn an_instance_of_a_module_template_reads_vislox_s_blocks() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let lock = Lock::default();
        let t = Module::parse_fragment(
            "template Paar(label: string = \"P\", quelle: input, grenze: number = 28) -> (Q: g.Q)\n\tg = GreaterEqual(\"{label}\", In1: quelle, In2: grenze)\nend\n",
        )
        .unwrap();
        let Item::Template(t) = t.items.into_iter().next().unwrap() else {
            panic!()
        };
        let templates: BTreeMap<String, logic::TemplateDecl> = [(t.name.clone(), t)].into();
        let mut ctx = ctx_of(&d, &d, &lock, None, &names);
        ctx.templates = &templates;
        apply_op(
            &mut files,
            &ctx,
            &sig("vx_warm", Some(&format!("{}.AQ >= 3", slug(DIV))), None),
        )
        .unwrap();
        apply_op(
            &mut files,
            &ctx,
            &inst(
                "vx_p",
                "Paar",
                &[("quelle", "vx_warm.Q and vx_warm.Q"), ("grenze", "30")],
                Some("Paar 1"),
            ),
        )
        .unwrap();
        let texts = files.render();
        assert!(
            !texts.contains_key(TEMPLATES),
            "the module's template is not copied"
        );
        let page = texts.values().find(|t| t.contains("vx_p =")).unwrap();
        assert!(
            page.contains("vx_p = Paar(\"Paar 1\", quelle: vx_warm.Q and vx_warm.Q, grenze: 30)"),
            "{page}"
        );
        // What reads the signal: its instance; dropping it is refused.
        let e = apply_op(&mut files.clone(), &ctx, &sig("vx_warm", None, None)).unwrap_err();
        assert!(e.contains("the instance vx_p"), "{e}");
        // A text parameter: the value given without quotes; a value: its kind.
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &inst("vx_p", "Paar", &[("grenze", "5min")], None),
        )
        .unwrap_err();
        assert!(e.contains("number parameter"), "{e}");
    }

    // ── expressions and named signals (#82) ─────────────────────────────

    fn expr(uuid: &str, port: &str, v: Option<&str>) -> Op {
        Op::Expr {
            uuid: uuid.into(),
            port: port.into(),
            value: v.map(Into::into),
        }
    }

    fn sig(slug: &str, v: Option<&str>, title: Option<&str>) -> Op {
        Op::Signal {
            slug: slug.into(),
            page: OG.into(),
            value: v.map(Into::into),
            title: title.map(Into::into),
        }
    }

    #[test]
    fn an_expression_onto_an_input_is_written_compiled_and_undone() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let div = slug(DIV);
        let vi = slug(VI);
        let text = format!("{div}.AQ >= 28 and not {vi}");
        let a = apply_op(&mut files, &ctx, &expr(AND, "I2", Some(&text))).unwrap();
        assert_eq!(a.previous, serde_json::Value::Null);
        let claims = files.render()[CLAIMS].clone();
        assert!(
            claims.contains(&format!("{}.In2 <- {text}\n", slug(AND))),
            "{claims}"
        );
        // The operands' externs are written, and read back.
        assert!(
            claims.contains(&format!("extern {div} = Div(uuid: \"{DIV}\")")),
            "{claims}"
        );
        assert!(
            claims.contains(&format!("extern {vi} = VirtualIn(")),
            "{claims}"
        );
        let back = Files::from_texts(&files.render()).unwrap();
        assert_eq!(back.claims, files.claims);
        assert_eq!(back.ops(&lock), files.ops(&lock));
        assert!(files.ops(&lock).contains(&expr(AND, "I2", Some(&text))));
        // lox makes its blocks, expression-owned, named after the sink.
        let out = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        let p = format!("{}_i2__", slug(AND));
        let owned: Vec<&String> = l.objects.keys().filter(|s| s.starts_with(&p)).collect();
        assert_eq!(owned.len(), 3, "{owned:?}");
        let texts: Vec<(String, String)> = files
            .render()
            .into_iter()
            .map(|(n, t)| (format!("pages/vislox/{n}"), t))
            .collect();
        let exprs =
            crate::expressions::expressions(&texts, &lock, |f| f.starts_with("pages/vislox/"));
        assert_eq!(exprs.len(), 1);
        let e = &exprs[0];
        assert!(e.ours && e.kind == "wire" && e.text == text, "{e:?}");
        assert_eq!(e.blocks.len(), 3);
        let top = find(&out.xml.root, e.top.as_deref().unwrap()).unwrap();
        assert_eq!(top.attr("Type"), Some("And"));
        // lox labels it with the expression, a bare name read as its output.
        let resolved = format!("{div}.AQ >= 28 and not {vi}.Q");
        assert_eq!(
            top.attr_decoded("Title").as_deref(),
            Some(resolved.as_str())
        );
        // The deployed config is the base: the expression is new there.
        let mut exprs = exprs;
        crate::expressions::compare(&mut exprs, Some(&d), Some(&out));
        assert!(exprs[0].pending && exprs[0].deployed.is_none());
        crate::expressions::compare(&mut exprs, Some(&out), Some(&out));
        assert!(!exprs[0].pending, "{:?}", exprs[0].deployed);
        assert_eq!(exprs[0].deployed.as_ref().unwrap().blocks.len(), 3);
        // Changed: the previous text comes back, and undoes it.
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let new = format!("{div}.AQ >= 30 or not {vi}");
        let a = apply_op(&mut files, &ctx, &expr(AND, "I2", Some(&new))).unwrap();
        assert_eq!(a.previous, serde_json::json!(text));
        let undo = undo_ops(&[expr(AND, "I2", Some(&new))], &[a.previous], &[]).unwrap();
        assert_eq!(undo, [expr(AND, "I2", Some(&text))]);
        // Dropped: the externs nothing names go with it.
        apply_op(&mut files, &ctx, &expr(AND, "I2", None)).unwrap();
        assert!(
            files.claims.0.values().all(|c| c.is_empty()),
            "{:?}",
            files.claims
        );
    }

    #[test]
    fn an_expression_is_refused_where_lox_or_loxone_config_feeds_the_input() {
        let d = doc("house.Loxone");
        let names: BTreeSet<String> = ["temp".to_string()].into();
        let mut files = Files::default();
        let lock = Lock::default();
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let div = slug(DIV);
        // I1 is wired in Loxone Config.
        let e = apply_op(
            &mut files,
            &ctx,
            &expr(AND, "I1", Some(&format!("{div}.AQ > 1"))),
        )
        .unwrap_err();
        assert!(e.contains("wired already"), "{e}");
        // Not an input.
        let e = apply_op(
            &mut files,
            &ctx,
            &expr(AND, "Q", Some(&format!("{div}.AQ > 1"))),
        )
        .unwrap_err();
        assert!(e.contains("not an input"), "{e}");
        // The module's own names, and names of nothing.
        let e = apply_op(&mut files, &ctx, &expr(AND, "I2", Some("temp.AQ > 1"))).unwrap_err();
        assert!(e.contains("the module's own"), "{e}");
        let e = apply_op(&mut files, &ctx, &expr(AND, "I2", Some("vx_nix.AQ > 1"))).unwrap_err();
        assert!(e.contains("names nothing"), "{e}");
        // A bare port of an object is a wire to draw, not an expression.
        let e = apply_op(&mut files, &ctx, &expr(AND, "I2", Some(&div))).unwrap_err();
        assert!(e.contains("bare name"), "{e}");
        // A constant edited here first.
        apply_op(&mut files, &ctx, &param(AND, "I2", Some("1"))).unwrap();
        let e = apply_op(
            &mut files,
            &ctx,
            &expr(AND, "I2", Some(&format!("{div}.AQ > 1"))),
        )
        .unwrap_err();
        assert!(e.contains("constant edited here"), "{e}");
        // Nothing changed by the refusals.
        assert_eq!(files.claims.0.len(), 1);
        assert!(files.claims.0.values().all(|c| c.exprs.is_empty()));
    }

    #[test]
    fn a_named_signal_is_made_named_and_keeps_its_type() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let div = slug(DIV);
        let inline = format!("{div}.AQ >= 28 and {div}.AQ < 40");
        apply_op(&mut files, &ctx, &expr(AND, "I2", Some(&inline))).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        // Name this expression: the signal, and the input fed from it.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let req = [
            sig("vx_warm", Some(&inline), None),
            expr(AND, "I2", Some("vx_warm.Q")),
        ];
        let mut prev = Vec::new();
        for op in &req {
            prev.push(apply_op(&mut files, &ctx, op).unwrap().previous);
        }
        let page = files.pages.values().next().unwrap().render();
        assert!(page.contains(&format!("vx_warm = {inline}\n")), "{page}");
        assert!(files.render()[CLAIMS].contains(&format!("{}.In2 <- vx_warm.Q", slug(AND))));
        let back = Files::from_texts(&files.render()).unwrap();
        assert_eq!((&back.claims, &back.pages), (&files.claims, &files.pages));
        let out2 = compile_files(&d, &files, &mut l).unwrap();
        assert!(l.objects.contains_key("vx_warm") && !l.objects["vx_warm"].expr_owned);
        assert!(l.objects.contains_key("vx_warm__ge1") && l.objects["vx_warm__ge1"].expr_owned);
        // The inline expression's blocks are gone.
        assert!(
            !l.objects.keys().any(|s| s.contains("_i2__")),
            "{:?}",
            l.objects.keys()
        );
        let warm = l.objects["vx_warm"].uuid.clone();
        // Undone: the expression back on the input, the signal dropped.
        let undo = undo_ops(&req, &prev, &[]).unwrap();
        assert_eq!(undo[0], expr(AND, "I2", Some(&inline)));
        assert_eq!(undo[1], sig("vx_warm", None, None));
        // Its label, and the expression changed below the same top block.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out2, &d, &lock, None, &names);
        let wider = format!("{div}.AQ >= 20 and {div}.AQ < 40");
        let a = apply_op(
            &mut files,
            &ctx,
            &sig("vx_warm", Some(&wider), Some("Warm")),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_value::<Op>(a.previous).unwrap(),
            sig("vx_warm", Some(&inline), None)
        );
        let out3 = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(l.objects["vx_warm"].uuid, warm);
        let top = find(&out3.xml.root, &warm).unwrap();
        assert_eq!(top.attr_decoded("Title").as_deref(), Some("Warm"));
        // `and` to `or` would change the top block's type: refused.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out3, &d, &lock, None, &names);
        let e = apply_op(
            &mut files,
            &ctx,
            &sig(
                "vx_warm",
                Some(&format!("{div}.AQ >= 20 or {div}.AQ < 4")),
                None,
            ),
        )
        .unwrap_err();
        assert!(e.contains("lox keeps a block's type"), "{e}");
        // Dropping it is refused while the input reads it, and once deployed.
        let e = apply_op(&mut files, &ctx, &sig("vx_warm", None, None)).unwrap_err();
        assert!(e.contains("reads it"), "{e}");
        apply_op(&mut files, &ctx, &expr(AND, "I2", None)).unwrap();
        let ctx = ctx_of(&out3, &d, &lock, Some(&out3), &names);
        let e = apply_op(&mut files.clone(), &ctx, &sig("vx_warm", None, None)).unwrap_err();
        assert!(e.contains("deployed"), "{e}");
        // Not deployed: dropped, with `removed` for lox; made again, it is back.
        let ctx = ctx_of(&out3, &d, &lock, Some(&d), &names);
        let a = apply_op(&mut files, &ctx, &sig("vx_warm", None, None)).unwrap();
        assert!(
            files
                .render()
                .values()
                .any(|t| t.contains("removed vx_warm"))
        );
        assert!(
            files
                .render()
                .values()
                .any(|t| t.contains(crate::blocks::DROPPED_LINE))
        );
        // Read back, it is still a dropped signal's line, which making it again takes.
        files = Files::from_texts(&files.render()).unwrap();
        assert!(files.pages.values().any(|p| p.dropped.contains("vx_warm")));
        apply_op(
            &mut files,
            &ctx,
            &serde_json::from_value(a.previous).unwrap(),
        )
        .unwrap();
        assert!(
            !files
                .render()
                .values()
                .any(|t| t.contains("removed vx_warm"))
        );
        compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(l.objects["vx_warm"].uuid, warm);
    }

    #[test]
    fn a_signal_name_is_checked() {
        let d = doc("house.Loxone");
        let names: BTreeSet<String> = ["vx_taken".to_string()].into();
        let mut files = Files::default();
        let lock = Lock::default();
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let e = format!("{}.AQ > 1", slug(DIV));
        for (s, why) in [
            ("warm", "named vx_"),
            (&slug(DIV) as &str, "named vx_"),
            ("vx_a__b", "`__`"),
            ("vx_taken", "taken"),
        ] {
            let got = apply_op(&mut files, &ctx, &sig(s, Some(&e), None)).unwrap_err();
            assert!(got.contains(why), "{s}: {got}");
        }
        let got = apply_op(&mut files, &ctx, &sig("vx_x", Some(&slug(DIV)), None)).unwrap_err();
        assert!(got.contains("alias"), "{got}");
        let got = apply_op(&mut files, &ctx, &sig("vx_x", None, None)).unwrap_err();
        assert!(got.contains("no signal"), "{got}");
        assert_eq!(files, Files::default());
    }

    #[test]
    fn what_an_expression_reads_stays_and_its_own_claim_always_drops() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        apply_op(&mut files, &ctx, &add(OG, "Monoflop", None, &[], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let mono = l.objects["vx_monoflop_1"].uuid.clone();
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let text = format!("vx_monoflop_1.Q and {}.AQ > 1", slug(DIV));
        apply_op(&mut files, &ctx, &expr(AND, "I2", Some(&text))).unwrap();
        compile_files(&d, &files, &mut l).unwrap();
        // The block the expression reads is not deleted from under it.
        let why = delete_refusal(&files, &ctx, &mono).unwrap();
        assert!(why.contains("reads it"), "{why}");
        // Once the module manages the sink (or it is gone), vislox's claim still drops.
        let mut managed = lock.clone();
        managed
            .slugs
            .insert("gate".into(), AND.to_ascii_uppercase());
        let ctx = ctx_of(&out, &d, &managed, None, &names);
        let a = apply_op(
            &mut files,
            &ctx,
            &expr(&AND.to_ascii_uppercase(), "I2", None),
        )
        .unwrap();
        assert_eq!(a.previous, serde_json::json!(text));
        assert!(files.claims.0.values().all(|c| c.exprs.is_empty()));
        assert!(delete_refusal(&files, &ctx_of(&out, &d, &lock, None, &names), &mono).is_none());
    }

    #[test]
    fn a_removed_line_of_a_deleted_block_is_not_taken_back_by_a_signal() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        apply_op(&mut files, &ctx, &add(OG, "And", None, &[], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let gate = l.objects["vx_and_1"].uuid.clone();
        // Deleted (deployed: `removed vx_and_1`, the deletion lox carries out).
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, Some(&out), &names);
        apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: gate }).unwrap();
        assert!(
            files
                .render()
                .values()
                .any(|t| t.contains("removed vx_and_1"))
        );
        // A signal of that name and type would cancel it: refused, nothing changed.
        let before = files.clone();
        let e = apply_op(
            &mut files,
            &ctx,
            &sig(
                "vx_and_1",
                Some(&format!("{0}.AQ > 1 and {0}.AQ < 9", slug(DIV))),
                None,
            ),
        )
        .unwrap_err();
        assert!(e.contains("taken"), "{e}");
        assert_eq!(files, before);
        // The `removed` line of a dropped signal is written apart, and read back as one.
        let page = files.pages.values().next().unwrap().render();
        assert!(!page.contains(crate::blocks::DROPPED_LINE), "{page}");
    }

    #[test]
    fn a_new_signal_feeds_an_input_whose_extern_was_dropped() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let lock = Lock::default();
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        apply_op(
            &mut files,
            &ctx,
            &sig("vx_an", Some(&format!("{}.AQ > 1", slug(DIV))), None),
        )
        .unwrap();
        apply_op(&mut files, &ctx, &expr(AND, "I2", Some("vx_an"))).unwrap();
        // The sink's extern is declared with the wire, and the files read back.
        assert!(
            files.claims.0.get(AND).is_some_and(|c| c.ty == "And"),
            "{:?}",
            files.claims
        );
        Files::from_texts(&files.render()).unwrap();
    }

    #[test]
    fn a_port_fed_into_an_input_is_checked_as_a_wire() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let lock = Lock::default();
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let div = slug(DIV);
        // An input is no source.
        let e = apply_op(
            &mut files,
            &ctx,
            &expr(AND, "I2", Some(&format!("{div}.Input2"))),
        )
        .unwrap_err();
        assert!(e.contains("is an input"), "{e}");
        assert_eq!(files, Files::default());
        // An output is: written as the wire op writes it.
        apply_op(
            &mut files,
            &ctx,
            &expr(AND, "I2", Some(&format!("{div}.AQ"))),
        )
        .unwrap();
        assert!(
            files
                .claims
                .1
                .iter()
                .any(|w| w.from == (div.clone(), "AQ".into()))
        );
        // Fed again from the same, it is the same wire.
        let a = apply_op(
            &mut files,
            &ctx,
            &expr(AND, "I2", Some(&format!("{div}.AQ"))),
        )
        .unwrap();
        assert_eq!(a.previous, serde_json::json!(format!("{div}.AQ")));
    }

    #[test]
    fn a_labelled_signal_is_pending_when_its_expression_changes() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let div = slug(DIV);
        apply_op(
            &mut files,
            &ctx,
            &sig(
                "vx_warm",
                Some(&format!("{div}.AQ >= 28 and {div}.AQ < 40")),
                Some("Warm"),
            ),
        )
        .unwrap();
        let deployed = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        let ctx = ctx_of(&deployed, &d, &lock, None, &names);
        apply_op(
            &mut files,
            &ctx,
            &sig(
                "vx_warm",
                Some(&format!("{div}.AQ >= 20 and {div}.AQ < 40")),
                Some("Warm"),
            ),
        )
        .unwrap();
        let pending = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        let texts: Vec<(String, String)> = files
            .render()
            .into_iter()
            .map(|(n, t)| (format!("pages/vislox/{n}"), t))
            .collect();
        let mut exprs = crate::expressions::expressions(&texts, &lock, |_| true);
        crate::expressions::compare(&mut exprs, Some(&deployed), Some(&pending));
        let e = &exprs[0];
        // The label is the same both sides; the chain below it is not.
        assert_eq!(e.deployed.as_ref().unwrap().text, "Warm");
        assert_eq!(e.now.as_deref(), Some("Warm"));
        assert!(e.pending, "{e:?}");
        crate::expressions::compare(&mut exprs, Some(&pending), Some(&pending));
        assert!(!exprs[0].pending);
    }

    // ── objects of the object tree (#84) ────────────────────────────────

    const VI_FOLDER: &str = "10000000-0000-0002-ffff504f94000001";
    const VO_FOLDER: &str = "10000000-0000-0003-ffff504f94000001";
    const NET_FOLDER: &str = "10000000-0000-0004-ffff504f94000001";
    /// A Modbus server Loxone Config made, in Netzwerkperipherie.
    const WP: &str = "10000000-0000-0005-ffff504f94000001";

    /// The house with a Virtual Outputs folder and a Netzwerkperipherie
    /// folder holding a Modbus server (made-up address).
    fn periphery_house() -> LoxoneDoc {
        let p = format!("{}/tests/fixtures/house.Loxone", env!("CARGO_MANIFEST_DIR"));
        let s = std::fs::read_to_string(p).unwrap();
        let add = format!(
            "\t\t<C Type=\"VirtualOutCaption\" V=\"178\" U=\"{VO_FOLDER}\" Title=\"Virtuelle Ausgänge\">\n\t\t</C>\n\
             \t\t<C Type=\"WeatherCaption\" V=\"178\" U=\"{NET_FOLDER}\" Title=\"Netzwerkperipherie\">\n\
             \t\t\t<C Type=\"ModbusServer\" V=\"178\" U=\"{WP}\" Title=\"WP\" Cl=\"0,0,0\" WF=\"16384\" Address=\"192.0.2.12:502\" Timeout=\"2000\">\n\
             \t\t\t</C>\n\t\t</C>\n"
        );
        let i = s.find("\t\t<C Type=\"ModeCaption\"").unwrap();
        let s = format!("{}{add}{}", &s[..i], &s[i..]);
        LoxoneDoc::parse(s.as_bytes()).unwrap()
    }

    /// The element `uuid`'s parent's UUID.
    fn parent_of(doc: &LoxoneDoc, uuid: &str) -> Option<String> {
        fn walk(el: &Element, uuid: &str) -> Option<String> {
            for c in el.child_elements() {
                if c.attr("U").is_some_and(|u| u.eq_ignore_ascii_case(uuid)) {
                    return el.attr("U").map(String::from);
                }
                if let Some(r) = walk(c, uuid) {
                    return Some(r);
                }
            }
            None
        }
        walk(&doc.xml.root, uuid)
    }

    #[test]
    fn objects_of_the_object_tree_are_added_into_their_folder_or_device() {
        let d = periphery_house();
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        for op in [
            add(
                VI_FOLDER,
                "VirtualIn",
                Some("Sollwert"),
                &[],
                &[
                    ("Analog", "on"),
                    ("MinVal", "5"),
                    ("MaxVal", "30"),
                    ("DefVal", "21"),
                ],
            ),
            add(
                VI_FOLDER,
                "VirtualHttpIn",
                Some("Zähler"),
                &[],
                &[("Address", "http://192.0.2.10/status"), ("Poll", "1min")],
            ),
            add(
                NET_FOLDER,
                "ModbusServer",
                Some("Lüftung"),
                &[],
                &[("Address", "192.0.2.13:502")],
            ),
            add(WP, "ModbusDev", Some("Regler"), &[], &[]),
        ] {
            apply_op(&mut files, &ctx, &op).unwrap();
        }
        let r = files.render();
        let text = &r[crate::periphery::FILE];
        assert!(!text.contains("page "), "{text}");
        assert!(
            text.contains(
                "vx_virtualhttpin_1 = VirtualHttpIn(\"Zähler\", Address: \"http://192.0.2.10/status\", Poll: 1min)\n"
            ),
            "{text}"
        );
        let wp = slug(WP);
        assert!(
            text.contains(&format!(
                "vx_modbusdev_1 = ModbusDev(\"Regler\", device: {wp})\n"
            )),
            "{text}"
        );
        assert!(
            r[CLAIMS].contains(&format!(
                "extern {wp} = ModbusServer(uuid: \"{}\")",
                WP.to_lowercase()
            )),
            "{r:?}"
        );
        // Read back as written.
        assert_eq!(Files::from_texts(&r).unwrap().pages, files.pages);
        let out = compile_files(&d, &files, &mut l).unwrap();
        let u = |l: &Lockfile, s: &str| l.objects[s].uuid.clone();
        assert_eq!(
            parent_of(&out, &u(&l, "vx_virtualin_1")).as_deref(),
            Some(VI_FOLDER)
        );
        assert_eq!(
            parent_of(&out, &u(&l, "vx_virtualhttpin_1")).as_deref(),
            Some(VI_FOLDER)
        );
        assert_eq!(
            parent_of(&out, &u(&l, "vx_modbusserver_1")).as_deref(),
            Some(NET_FOLDER)
        );
        assert_eq!(
            parent_of(&out, &u(&l, "vx_modbusdev_1")).as_deref(),
            Some(WP)
        );
        let http = find(&out.xml.root, &u(&l, "vx_virtualhttpin_1")).unwrap();
        assert_eq!(http.attr("Poll"), Some("60"));
        assert_eq!(http.attr("IName"), Some("VHI1"));
        assert!(lock_of(&l).placed.is_empty(), "no position");

        // Its I/O: below the device just made, and below the one made here.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let http_u = u(&l, "vx_virtualhttpin_1");
        let dev_u = u(&l, "vx_modbusdev_1");
        for op in [
            add(
                &http_u,
                "VirtualHttpInCmd",
                Some("Leistung"),
                &[],
                &[("Check", "\"power\":\\v")],
            ),
            add(
                &dev_u,
                "ModbusASensor",
                Some("Vorlauf"),
                &[],
                &[("ModbusAddress", "1"), ("MaxTime", "46s")],
            ),
        ] {
            apply_op(&mut files, &ctx, &op).unwrap();
        }
        let text = &files.render()[crate::periphery::FILE];
        assert!(
            text.contains(
                "vx_virtualhttpincmd_1 = VirtualHttpInCmd(\"Leistung\", device: vx_virtualhttpin_1, Check: \"\\\"power\\\":\\\\v\")\n"
            ),
            "{text}"
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        let cmd = find(&out.xml.root, &u(&l, "vx_virtualhttpincmd_1")).unwrap();
        assert_eq!(cmd.attr_decoded("Check").as_deref(), Some("\"power\":\\v"));
        assert_eq!(
            parent_of(&out, &u(&l, "vx_virtualhttpincmd_1")),
            Some(http_u.clone())
        );
        let sensor = find(&out.xml.root, &u(&l, "vx_modbusasensor_1")).unwrap();
        assert_eq!(sensor.attr("MaxTime"), Some("46000"));
        assert_eq!(
            parent_of(&out, &u(&l, "vx_modbusasensor_1")),
            Some(dev_u.clone())
        );
        // What vislox's files set, listed and revertable; never where it is.
        let ops = files.ops(&lock_of(&l));
        assert!(
            ops.contains(&Op::Setting {
                uuid: http_u.clone(),
                name: "Poll".into(),
                value: Some("1min".into())
            }),
            "{ops:?}"
        );
        assert!(
            !ops.iter()
                .any(|o| matches!(o, Op::Setting { name, .. } if name == "device")),
            "{ops:?}"
        );
    }

    #[test]
    fn an_object_is_refused_without_what_lox_needs_and_nothing_changes() {
        let d = periphery_house();
        let names = BTreeSet::new();
        let files = Files::default();
        let lock = Lock::default();
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        for (op, why) in [
            (
                add(
                    VI_FOLDER,
                    "VirtualHttpIn",
                    None,
                    &[],
                    &[("Address", "http://x")],
                ),
                "needs Poll",
            ),
            (
                add(
                    VI_FOLDER,
                    "VirtualHttpIn",
                    None,
                    &[],
                    &[("Address", "http://x"), ("Poll", "5")],
                ),
                "from 10 to 604800",
            ),
            (
                add(
                    VI_FOLDER,
                    "VirtualHttpIn",
                    None,
                    &[],
                    &[("Address", " "), ("Poll", "60")],
                ),
                "cannot be empty",
            ),
            (
                add(VI_FOLDER, "VirtualIn", None, &[], &[("MaxVal", "30")]),
                "would write 1",
            ),
            (
                add(VI_FOLDER, "VirtualIn", None, &[], &[("IName", "VI9")]),
                "IName cannot be set",
            ),
            (
                add(VI_FOLDER, "VirtualHttpInCmd", None, &[], &[("Check", "x")]),
                "hangs below a VirtualHttpIn",
            ),
            (
                add(VO_FOLDER, "VirtualIn", None, &[], &[]),
                "goes into the folder Virtual Inputs",
            ),
            (
                add(WP, "ModbusASensor", None, &[], &[("ModbusAddress", "1")]),
                "hangs below a ModbusDev",
            ),
            (
                add(
                    VO_FOLDER,
                    "VirtualOut",
                    None,
                    &[("I", "1")],
                    &[("Address", "x")],
                ),
                "takes no constants",
            ),
            (add(EG, "VirtualIn", None, &[], &[]), "goes into the folder"),
            (
                add(
                    "deadbeef-0000-0000-ffff504f94000001",
                    "ModbusDev",
                    None,
                    &[],
                    &[],
                ),
                "no object",
            ),
            // A unit only where lox documents one, and checked in it.
            (
                add(VI_FOLDER, "VirtualUdpIn", None, &[], &[("Port", "30min")]),
                "plain number",
            ),
            (
                add(
                    VI_FOLDER,
                    "VirtualHttpIn",
                    None,
                    &[],
                    &[("Address", "x"), ("Poll", "5s")],
                ),
                "write 10 s",
            ),
            (
                add(
                    VI_FOLDER,
                    "VirtualHttpIn",
                    None,
                    &[],
                    &[("Address", "x"), ("Poll", "200h")],
                ),
                "write 604800 s",
            ),
            (
                add(WP, "ModbusDev", None, &[], &[("Channel", "70%")]),
                "plain number",
            ),
            // Text by type: a virtual output's command separator is text, its
            // CloseAfterSend on or off.
            (
                add(
                    VO_FOLDER,
                    "VirtualOut",
                    None,
                    &[],
                    &[("Address", "x"), ("CloseAfterSend", "maybe")],
                ),
                "on or off",
            ),
        ] {
            let mut f = files.clone();
            let e = apply_op(&mut f, &ctx, &op).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
            assert_eq!(f, files);
        }
        // An object of the object tree is not a page block, nor a page block one.
        let e = Files::from_texts(&BTreeMap::from([(
            "erdgeschoss.loxl".to_string(),
            "page \"Erdgeschoss\"\nvx_virtualin_1 = VirtualIn()\n".to_string(),
        )]))
        .unwrap_err();
        assert!(e.contains("vislox did not write"), "{e}");
        let e = Files::from_texts(&BTreeMap::from([(
            crate::periphery::FILE.to_string(),
            "vx_and_1 = And()\n".to_string(),
        )]))
        .unwrap_err();
        assert!(e.contains("vislox did not write"), "{e}");
    }

    #[test]
    fn an_objects_settings_are_edited_and_it_is_taken_out_and_deleted_like_a_block() {
        let d = periphery_house();
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        apply_op(
            &mut files,
            &ctx,
            &add(
                VI_FOLDER,
                "VirtualUdpIn",
                Some("UDP"),
                &[],
                &[("Port", "7000")],
            ),
        )
        .unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let udp = l.objects["vx_virtualudpin_1"].uuid.clone();
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        apply_op(
            &mut files,
            &ctx,
            &add(
                &udp,
                "VirtualUdpInCmd",
                Some("T"),
                &[],
                &[("Check", "t=\\v")],
            ),
        )
        .unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let cmd = l.objects["vx_virtualudpincmd_1"].uuid.clone();
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let set = |name: &str, v: Option<&str>| Op::Setting {
            uuid: cmd.clone(),
            name: name.into(),
            value: v.map(Into::into),
        };
        // Its settings: changed, their undo the value before; a required one
        // stays; one a save rewrites is refused.
        let r = apply_op(&mut files, &ctx, &set("MaxTime", Some("30s"))).unwrap();
        assert_eq!(r.previous, serde_json::Value::Null);
        let r = apply_op(&mut files, &ctx, &set("Check", Some("v=\\v"))).unwrap();
        assert_eq!(r.previous, serde_json::json!("t=\\v"));
        let e = apply_op(&mut files.clone(), &ctx, &set("Check", None)).unwrap_err();
        assert!(e.contains("needs Check"), "{e}");
        let e = apply_op(&mut files.clone(), &ctx, &set("ValOT", Some("1.5"))).unwrap_err();
        assert!(e.contains("would write 1"), "{e}");
        apply_op(
            &mut files,
            &ctx,
            &Op::Title {
                uuid: cmd.clone(),
                value: Some("Temp".into()),
            },
        )
        .unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let el = find(&out.xml.root, &cmd).unwrap();
        assert_eq!(
            (el.attr("MaxTime"), el.attr("Title")),
            (Some("30000"), Some("Temp"))
        );

        // The device is not taken out while its command hangs below it.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let e = delete_refusal(&files, &ctx, &udp).unwrap();
        assert!(e.contains("Temp hangs below it"), "{e}");
        // The command, not deployed, is taken out; its undo adds it again below its device.
        let r = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: cmd.clone() }).unwrap();
        let again: Op = serde_json::from_value(r.previous).unwrap();
        let Op::AddBlock {
            ref page,
            ref settings,
            ..
        } = again
        else {
            panic!("{again:?}")
        };
        assert_eq!(page, &udp);
        assert_eq!(settings.get("Check").map(String::as_str), Some("v=\\v"));
        assert!(!settings.contains_key("device"));
        apply_op(&mut files, &ctx, &again).unwrap();

        // Deployed: deleted from the config, and back with its UUIDs.
        let deployed = compile_files(&d, &files, &mut l).unwrap();
        let base = deployed.clone();
        let cmd = l.objects["vx_virtualudpincmd_2"].uuid.clone();
        let was = identity(&deployed, &cmd);
        let lock = lock_of(&l);
        let ctx = ctx_of(&deployed, &base, &lock, Some(&deployed), &names);
        let r = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: cmd.clone() }).unwrap();
        let again: Op = serde_json::from_value(r.previous).unwrap();
        let Op::RestoreBlock {
            ref page, ref lock, ..
        } = again
        else {
            panic!("{again:?}")
        };
        assert_eq!(page, &udp);
        assert!(lock["layout"].is_null());
        let gone = compile_files(&base, &files, &mut l).unwrap();
        assert!(find(&gone.xml.root, &cmd).is_none());
        let lock = lock_of(&l);
        files.tidy(&lock);
        let ctx = ctx_of(&gone, &base, &lock, Some(&deployed), &names);
        // Put elsewhere than its declaration says: refused.
        let mut moved = again.clone();
        if let Op::RestoreBlock { page, .. } = &mut moved {
            *page = VI_FOLDER.into();
        }
        let e = apply_op(&mut files.clone(), &ctx, &moved).unwrap_err();
        assert!(e.contains("elsewhere"), "{e}");
        apply_op(&mut files, &ctx, &again).unwrap();
        l = edit_lock(&l, &files.lock_edits);
        files.lock_edits = Default::default();
        let back = compile_files(&base, &files, &mut l).unwrap();
        assert_eq!(identity(&back, &cmd), was);
        assert!(loxone_config::diff::diff(&deployed, &back).logic_is_empty());

        // Renamed: what hangs below it follows.
        let lock = lock_of(&l);
        let ctx = ctx_of(&back, &base, &lock, Some(&deployed), &names);
        apply_op(
            &mut files,
            &ctx,
            &Op::Rename {
                uuid: udp.clone(),
                slug: "vx_udp".into(),
            },
        )
        .unwrap();
        let text = &files.render()[crate::periphery::FILE];
        assert!(text.contains("device: vx_udp,"), "{text}");
        assert!(text.contains("moved vx_virtualudpin_1 -> vx_udp"), "{text}");
        let renamed = compile_files(&base, &files, &mut l).unwrap();
        assert!(loxone_config::diff::diff(&back, &renamed).logic_is_empty());
    }

    #[test]
    fn a_device_is_not_deleted_with_what_loxone_config_hung_below_it() {
        let d = periphery_house();
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        apply_op(
            &mut files,
            &ctx,
            &add(
                VO_FOLDER,
                "VirtualOut",
                Some("Out"),
                &[],
                &[("Address", "http://192.0.2.20"), ("CmdSep", ";")],
            ),
        )
        .unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let vo = l.objects["vx_virtualout_1"].uuid.clone();
        // Deployed, then Loxone Config hung a command below it.
        let text = String::from_utf8(out.to_bytes()).unwrap();
        let at = text.find(&format!("U=\"{vo}\"")).unwrap();
        let end = at + text[at..].find('>').unwrap() + 1;
        let with = format!(
            "{}\n<C Type=\"VirtualOutCmd\" V=\"178\" U=\"a0000003-0000-0000-ffff504f94000001\" Title=\"Licht an\" CmdOn=\"/on\"><Co K=\"I\" U=\"a0000003-0000-0001-ffff504f94000001\"/></C>{}",
            &text[..end],
            &text[end..]
        );
        let base = LoxoneDoc::parse(with.as_bytes()).unwrap();
        let lock = lock_of(&l);
        let ctx = ctx_of(&base, &base, &lock, Some(&base), &names);
        let e = delete_refusal(&files, &ctx, &vo).unwrap();
        assert!(
            e.contains("Licht an below it was made in Loxone Config"),
            "{e}"
        );
        let mut f = files.clone();
        let e = apply_op(&mut f, &ctx, &Op::RemoveBlock { uuid: vo.clone() }).unwrap_err();
        assert!(e.contains("Licht an"), "{e}");
        assert_eq!(f, files);
        // Only in the base (the pending config lost it): refused all the same.
        let ctx = ctx_of(&out, &base, &lock, Some(&base), &names);
        assert!(
            delete_refusal(&files, &ctx, &vo)
                .unwrap()
                .contains("Licht an")
        );
        // A command of vislox's own below it is named as such.
        let ctx = ctx_of(&base, &base, &lock, Some(&base), &names);
        apply_op(
            &mut files,
            &ctx,
            &add(
                &vo,
                "VirtualOutCmd",
                Some("Aus"),
                &[],
                &[("CmdOff", "/off")],
            ),
        )
        .unwrap();
        assert!(
            delete_refusal(&files, &ctx, &vo)
                .unwrap()
                .contains("hangs below it")
        );
    }

    #[test]
    fn where_an_object_can_be_added_is_served_with_the_reason() {
        let d = periphery_house();
        let names = BTreeSet::new();
        let lock = Lock::default();
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let m = crate::periphery::add_refusals(&Files::default(), &ctx);
        assert_eq!(m[VI_FOLDER], serde_json::Value::Null);
        assert_eq!(m[WP], serde_json::Value::Null);
        assert!(!m.contains_key(EG), "a page takes none");
        // Two Virtual Outputs folders: lox adds to the one there is, so neither takes one.
        let text = String::from_utf8(d.to_bytes()).unwrap().replacen(
            "<C Type=\"VirtualOutCaption\"",
            "<C Type=\"VirtualOutCaption\" V=\"178\" U=\"a0000009-0000-0000-ffff504f94000001\" Title=\"Zwei\"></C>\n<C Type=\"VirtualOutCaption\"",
            1,
        );
        let two = LoxoneDoc::parse(text.as_bytes()).unwrap();
        let ctx = ctx_of(&two, &two, &lock, None, &names);
        let m = crate::periphery::add_refusals(&Files::default(), &ctx);
        assert!(
            m[VO_FOLDER].as_str().unwrap().contains("2 folders"),
            "{m:?}"
        );
        // A device not in the base: lox resolves an extern there.
        let plain = doc("house.Loxone");
        let ctx = ctx_of(&d, &plain, &lock, None, &names);
        let m = crate::periphery::add_refusals(&Files::default(), &ctx);
        assert!(
            m[WP]
                .as_str()
                .unwrap()
                .contains("not in the project's base"),
            "{m:?}"
        );
    }

    // ── Tree and Air hardware (#85) ────────────────────────────────────

    const HW_TREE: &str = "20000002-0000-0020-ffff504f94000002";
    const HW_TOUCH: &str = "20000003-0000-0030-ffff504f94000002";
    const HW_TEMP: &str = "20000003-0000-0031-ffff504f94000002";
    const HW_VOLT: &str = "20000005-0000-0051-ffff504f94000002";
    const HW_PAGE: &str = "20000010-0000-0100-ffff504f94000002";

    fn stats(uuid: &str, v: Option<&str>) -> Op {
        Op::Statistics {
            uuid: uuid.into(),
            value: v.map(String::from),
        }
    }

    fn stats_type(doc: &LoxoneDoc, uuid: &str) -> Option<String> {
        find_ci(&doc.xml.root, uuid).and_then(|e| e.attr("StatsType").map(String::from))
    }

    /// The UUID of the I/O `iname` below the device `dev`.
    fn io_of(doc: &LoxoneDoc, dev: &str, iname: &str) -> String {
        find_ci(&doc.xml.root, dev)
            .unwrap()
            .child_elements()
            .find(|c| c.attr("IName") == Some(iname))
            .and_then(|c| c.attr("U"))
            .unwrap()
            .to_string()
    }

    #[test]
    fn statistics_are_a_hardware_declaration_lox_writes_and_drops() {
        let d = doc("hardware.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        assert_eq!(current(&d, &stats(HW_TEMP, None)).as_deref(), Some("off"));
        let r = apply_op(&mut files, &ctx, &stats(HW_TEMP, Some("on"))).unwrap();
        assert_eq!(r.previous, serde_json::Value::Null);
        let text = files.render()[crate::hw_edit::FILE].clone();
        let dev = crate::hw_edit::hw_slug(HW_TOUCH);
        assert!(
            text.contains(&format!("{dev} = TreeDevice(uuid: \"{HW_TOUCH}\")\n")),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "{} = TreeAsensor(device: {dev}, uuid: \"{HW_TEMP}\", Statistics: on)\n",
                crate::hw_edit::hw_slug(HW_TEMP)
            )),
            "{text}"
        );
        // Read back as written.
        let back = Files::from_texts(&files.render()).unwrap();
        assert_eq!(back.hardware, files.hardware);
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(stats_type(&out, HW_TEMP).as_deref(), Some("11"));
        assert_eq!(current(&out, &stats(HW_TEMP, None)).as_deref(), Some("on"));
        // The device keeps its name: the declaration states nothing else.
        assert_eq!(
            find_ci(&out.xml.root, HW_TOUCH)
                .unwrap()
                .attr_decoded("Title")
                .unwrap(),
            "Touch Küche"
        );
        let lock = lock_of(&l);
        assert!(files.ops(&lock).contains(&stats(HW_TEMP, Some("on"))));

        // Off removes it; the undo is the value before.
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let op = stats(HW_TEMP, Some("off"));
        let r = apply_op(&mut files, &ctx, &op).unwrap();
        assert_eq!(r.previous, serde_json::json!("on"));
        assert_eq!(
            undo_ops(&[op], &[r.previous], &[]).unwrap(),
            vec![stats(HW_TEMP, Some("on"))]
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(stats_type(&out, HW_TEMP), None);

        // Dropping the claim takes the declarations out: the base's again.
        let r = apply_op(&mut files, &ctx, &stats(HW_TEMP, None)).unwrap();
        assert_eq!(r.previous, serde_json::json!("off"));
        assert!(files.hardware.is_empty(), "{:?}", files.hardware);
        assert!(!files.render().contains_key(crate::hw_edit::FILE));

        // Refused, changing nothing: a type lox records no statistics on, a
        // device, a word lox does not take, an I/O the module declares.
        for (op, why) in [
            (stats(HW_VOLT, Some("on")), "only on a Tree or Air"),
            (stats(HW_TOUCH, Some("on")), "only on a Tree or Air"),
            (stats(HW_TEMP, Some("yes")), "on or off"),
        ] {
            let mut f = files.clone();
            let e = apply_op(&mut f, &ctx, &op).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
            assert_eq!(f, files);
        }
        let mut own = lock.clone();
        own.externals.insert("taster".into(), HW_TOUCH.into());
        own.owned.insert("taster".into());
        let ctx = ctx_of(&out, &d, &own, None, &names);
        let e = apply_op(&mut files.clone(), &ctx, &stats(HW_TEMP, Some("on"))).unwrap_err();
        assert!(e.contains("`taster`"), "{e}");
        // A claim is always dropped, whoever else declares it.
        apply_op(&mut files.clone(), &ctx, &stats(HW_TEMP, None)).unwrap();
    }

    #[test]
    fn where_a_tree_device_can_be_added_is_served_with_the_reason() {
        let d = doc("hardware.Loxone");
        let names = BTreeSet::new();
        let lock = Lock::default();
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let m = crate::hw_edit::add_refusals(&ctx);
        assert_eq!(m.len(), 1);
        assert_eq!(m[HW_TREE], serde_json::Value::Null);
        // A Tree extension the base does not have yet: lox resolves the extern there.
        let house = doc("house.Loxone");
        let ctx = ctx_of(&d, &house, &lock, None, &names);
        let m = crate::hw_edit::add_refusals(&ctx);
        assert!(
            m[HW_TREE].as_str().unwrap().contains("Apply it first"),
            "{m:?}"
        );
        // The palette offers it on a Tree extension, with the kinds lox creates.
        let p = crate::hw_edit::palette();
        assert_eq!(p["devices"], serde_json::json!(["LoxTree"]));
        assert_eq!(p["settings"][0]["choices"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn a_new_device_s_input_is_claimed_and_dropped_in_one_request_by_its_designation() {
        let d = doc("hardware.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        apply_op(
            &mut files,
            &ctx,
            &add(HW_TREE, "TreeDevice", None, &[], &[("kind", "TouchTree")]),
        )
        .unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let dev = l.externals["vx_touchtree_1"].uuid.clone();
        let ai1 = io_of(&out, &dev, "AI1");
        // The lock does not know the I/O's declaration yet: on, off, then
        // Revert in one request each see the one before.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let mut f = files.clone();
        let a = apply_op(&mut f, &ctx, &stats(&ai1, Some("on"))).unwrap();
        assert_eq!(a.previous, serde_json::Value::Null);
        let b = apply_op(&mut f, &ctx, &stats(&ai1, Some("off"))).unwrap();
        assert_eq!(b.previous, serde_json::json!("on"));
        assert_eq!(f.hardware.ios.len(), 1);
        let c = apply_op(&mut f, &ctx, &stats(&ai1, None)).unwrap();
        assert_eq!(c.previous, serde_json::json!("off"));
        assert!(f.hardware.ios.is_empty(), "{:?}", f.hardware.ios);
        // And on again: not "taken" by the claim dropped a moment ago.
        apply_op(&mut f, &ctx, &stats(&ai1, Some("on"))).unwrap();
        assert_eq!(f.hardware.ios[0].slug, "vx_touchtree_1_ai1");
        assert_eq!(f.hardware.ios[0].ty, "TreeAsensor");

        // A designation that is no identifier is refused, as add_block would.
        let xml = String::from_utf8(out.to_bytes())
            .unwrap()
            .replace("IName=\"AI1\"", "IName=\"A-1\"");
        let odd = LoxoneDoc::parse(xml.as_bytes()).unwrap();
        let ctx = ctx_of(&odd, &d, &lock, None, &names);
        let e = apply_op(&mut files.clone(), &ctx, &stats(&ai1, Some("on"))).unwrap_err();
        assert!(e.contains("letters and digits"), "{e}");

        // Its undo carries each I/O's type: one left out, or another, is refused.
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        for (settings, why) in [
            (vec![("I1.Statistics", "on")], "go together"),
            (
                vec![("I1.Statistics", "on"), ("I1.Type", "VoltageIn")],
                "not \"VoltageIn\"",
            ),
            (vec![("I-1.Statistics", "on")], "cannot be set here"),
        ] {
            let mut all = vec![("kind", "TouchTree")];
            all.extend(settings);
            let e = apply_op(
                &mut files.clone(),
                &ctx,
                &add(HW_TREE, "TreeDevice", None, &[], &all),
            )
            .unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
        }
        // A restore is a Tree device's only by the lockfile's tombstone.
        assert!(!crate::hw_edit::restores_device(&ctx, &dev));
    }

    #[test]
    fn the_hardware_file_takes_only_what_vislox_writes() {
        for text in [
            "a = And()\n",
            "extern x = LoxTree(serial: \"13000001\")\n",
            "vx_x = TreeDevice(\"T\", serial: \"B0\")\n",
            "vx_hw_1 = TreeDevice(uuid: \"1\")\nvx_y = TreeSensor(device: vx_hw_1, uuid: \"2\", MinTime: 5)\n",
            "vx_y = TreeSensor(device: vx_nope, iname: \"I1\", Statistics: on)\n",
            "vx_d = TreeDevice(kind: \"TouchTree\", tree: vx_hw_nope)\n",
        ] {
            assert!(crate::hw_edit::HwFile::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn a_tree_device_is_created_unpaired_and_taken_out_or_deleted() {
        let d = doc("hardware.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let new = add(
            HW_TREE,
            "TreeDevice",
            Some("Melder Flur"),
            &[],
            &[("kind", "PresenceSensorTree"), ("branch", "left")],
        );
        // Refused, changing nothing: a kind lox does not create, a branch it
        // does not know, somewhere that is no Tree extension, a setting.
        for (op, why) in [
            (
                add(HW_TREE, "TreeDevice", None, &[], &[("kind", "Banana")]),
                "cannot create a Banana",
            ),
            (
                add(
                    HW_TREE,
                    "TreeDevice",
                    None,
                    &[],
                    &[("kind", "TouchTree"), ("branch", "middle")],
                ),
                "left or right",
            ),
            (
                add(HW_TOUCH, "TreeDevice", None, &[], &[("kind", "TouchTree")]),
                "no Tree extension",
            ),
            (
                add(
                    HW_TREE,
                    "TreeDevice",
                    None,
                    &[],
                    &[("kind", "TouchTree"), ("Serial", "1")],
                ),
                "cannot be set here",
            ),
        ] {
            let mut f = files.clone();
            let e = apply_op(&mut f, &ctx, &op).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
            assert_eq!(f, files);
        }
        let r = apply_op(&mut files, &ctx, &new).unwrap();
        assert_eq!(r.added.as_deref(), Some("vx_presencesensortree_1"));
        let text = files.render()[crate::hw_edit::FILE].clone();
        let tree = crate::hw_edit::hw_slug(HW_TREE);
        assert!(
            text.contains(&format!(
                "vx_presencesensortree_1 = TreeDevice(\"Melder Flur\", kind: \
                 \"PresenceSensorTree\", tree: {tree}, branch: \"left\")\n"
            )),
            "{text}"
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        let dev = l.externals["vx_presencesensortree_1"].uuid.clone();
        let el = find_ci(&out.xml.root, &dev).unwrap();
        assert_eq!(el.attr("Serial"), Some("00000000"));
        assert_eq!(el.attr_decoded("Title").unwrap(), "Melder Flur");

        // Its statistics by its designation: lox mints its I/O anew on each compile.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let i1 = io_of(&out, &dev, "I1");
        apply_op(&mut files, &ctx, &stats(&i1, Some("on"))).unwrap();
        assert!(files.render()[crate::hw_edit::FILE].contains(
            "vx_presencesensortree_1_i1 = TreeSensor(device: vx_presencesensortree_1, \
                 iname: \"I1\", Statistics: on)\n"
        ));
        // Its name is its declaration's; the rest waits for Apply.
        apply_op(
            &mut files,
            &ctx,
            &Op::Title {
                uuid: dev.clone(),
                value: Some("Melder Diele".into()),
            },
        )
        .unwrap();
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &Op::Title {
                uuid: i1.clone(),
                value: Some("Bewegung".into()),
            },
        )
        .unwrap_err();
        assert!(e.contains("not in the project's base"), "{e}");
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(
            find_ci(&out.xml.root, &dev)
                .unwrap()
                .attr_decoded("Title")
                .unwrap(),
            "Melder Diele"
        );
        assert_eq!(
            stats_type(&out, &io_of(&out, &dev, "I1")).as_deref(),
            Some("7")
        );

        // Not deployed: taken out, and added again by its undo with what it had.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let r = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: dev.clone() }).unwrap();
        let again: Op = serde_json::from_value(r.previous).unwrap();
        let mut want = add(
            HW_TREE,
            "TreeDevice",
            Some("Melder Diele"),
            &[],
            &[
                ("kind", "PresenceSensorTree"),
                ("branch", "left"),
                ("I1.Statistics", "on"),
                ("I1.Type", "TreeSensor"),
            ],
        );
        if let Op::AddBlock { restores, .. } = &mut want {
            *restores = Some(dev.clone());
        }
        assert_eq!(again, want);
        assert!(files.hardware.devices.is_empty(), "{:?}", files.hardware);
        assert!(files.hardware.removed.contains("vx_presencesensortree_1"));
        let gone = compile_files(&d, &files, &mut l).unwrap();
        assert!(find_ci(&gone.xml.root, &dev).is_none());
        let lock = lock_of(&l);
        let ctx = ctx_of(&gone, &d, &lock, None, &names);
        // The `removed` line is done: lox forgot the device.
        assert_eq!(
            files.stale_lines(&lock),
            ["removed vx_presencesensortree_1"]
        );
        files.tidy(&lock);
        let r = apply_op(&mut files, &ctx, &again).unwrap();
        assert_eq!(r.added.as_deref(), Some("vx_presencesensortree_1"));
        let out = compile_files(&d, &files, &mut l).unwrap();
        let dev = l.externals["vx_presencesensortree_1"].uuid.clone();

        // Deployed, with a wire Loxone Config drew from it into a block:
        // refused, as lox would keep the device.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        apply_op(&mut files, &ctx, &add(HW_PAGE, "And", None, &[], &[])).unwrap();
        let base = compile_files(&d, &files, &mut l).unwrap();
        let and = l.objects["vx_and_1"].uuid.clone();
        let q = find_ci(&base.xml.root, &io_of(&base, &dev, "I1"))
            .unwrap()
            .child_elements()
            .find(|c| c.attr("K") == Some("Q"))
            .and_then(|c| c.attr("U"))
            .unwrap()
            .to_string();
        let i1 = find_ci(&base.xml.root, &and)
            .unwrap()
            .child_elements()
            .find(|c| c.attr("K") == Some("I1"))
            .and_then(|c| c.attr("U"))
            .unwrap()
            .to_string();
        let xml = String::from_utf8(base.to_bytes()).unwrap().replace(
            &format!("<Co K=\"I1\" U=\"{i1}\"/>"),
            &format!("<Co K=\"I1\" U=\"{i1}\"><In Input=\"{q}\"/></Co>"),
        );
        let drawn = LoxoneDoc::parse(xml.as_bytes()).unwrap();
        assert_eq!(drawn.wires().len(), 1);
        let lock = lock_of(&l);
        let ctx = ctx_of(&drawn, &drawn, &lock, Some(&drawn), &names);
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &Op::RemoveBlock { uuid: dev.clone() },
        )
        .unwrap_err();
        assert!(e.contains("wired to a block"), "{e}");
        assert_eq!(
            delete_refusals(&files, &ctx, std::slice::from_ref(&dev))[0].as_deref(),
            Some(e.as_str())
        );
        let was = identity(&base, &dev);

        // Deleted with `removed`, and brought back under its UUID.
        let lock = lock_of(&l);
        let ctx = ctx_of(&base, &base, &lock, Some(&base), &names);
        assert_eq!(
            delete_refusals(&files, &ctx, std::slice::from_ref(&dev))[0],
            None
        );
        let r = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: dev.clone() }).unwrap();
        let back: Op = serde_json::from_value(r.previous).unwrap();
        assert!(matches!(back, Op::RestoreBlock { .. }), "{back:?}");
        assert!(files.render()[crate::hw_edit::FILE].contains("removed vx_presencesensortree_1\n"));
        let gone = compile_files(&base, &files, &mut l).unwrap();
        assert!(find_ci(&gone.xml.root, &dev).is_none());
        let lock = lock_of(&l);
        assert_eq!(
            lock.tomb(&dev).map(|t| t.1),
            Some("vx_presencesensortree_1")
        );
        assert_eq!(
            files.stale_lines(&lock),
            ["removed vx_presencesensortree_1"]
        );
        files.tidy(&lock);
        // A forged restore is refused: another slug, another device.
        let ctx = ctx_of(&gone, &base, &lock, Some(&base), &names);
        let mut forged = back.clone();
        if let Op::RestoreBlock { source, .. } = &mut forged {
            *source = source.replace("vx_presencesensortree_1", "vx_other");
        }
        assert!(apply_op(&mut files.clone(), &ctx, &forged).is_err());
        let r = apply_op(&mut files, &ctx, &back).unwrap();
        assert_eq!(
            serde_json::from_value::<Op>(r.previous).unwrap(),
            Op::RemoveBlock { uuid: dev.clone() }
        );
        l = edit_lock(&l, &files.lock_edits);
        files.lock_edits = Default::default();
        let again = compile_files(&base, &files, &mut l).unwrap();
        assert_eq!(identity(&again, &dev), was);
        assert!(l.removed.is_empty(), "{:?}", l.removed);
    }

    // ── operating modes and calendar entries (#86) ──────────────────────

    const MODE_F: &str = "10000010-0000-0100-ffff504f94000001";
    const CAL_F: &str = "10000012-0000-0120-ffff504f94000001";
    const HEILIGABEND: &str = "10000013-0000-0130-ffff504f94000001";

    #[test]
    fn modes_and_calendar_entries_are_added_and_edited_as_lox_compiles_them() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        apply_op(
            &mut files,
            &ctx,
            &add(MODE_F, "Mode", Some("Grill"), &[], &[]),
        )
        .unwrap();
        // A mode takes no settings, and goes into its folder only.
        assert!(
            apply_op(
                &mut files.clone(),
                &ctx,
                &add(CAL_F, "Mode", Some("X"), &[], &[])
            )
            .unwrap_err()
            .contains("Operating Modes")
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        let grill = l.objects["vx_mode_1"].uuid.clone();
        let el = find(&out.xml.root, &grill).unwrap();
        // One below the lowest in use (Party is -1).
        assert_eq!(el.attr("Mode"), Some("-2"));
        assert_eq!(parent_of(&out, &grill).as_deref(), Some(MODE_F));

        // Entries: of the mode just made (by slug), of a built-in mode (by its
        // day word) and of a mode Loxone Config made (by its title).
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let vacation = crate::sched_edit::builtin_uuid(2);
        let entry = |title: &str, mode: &str, date: &str| {
            add(
                CAL_F,
                "CalendarEntry",
                Some(title),
                &[],
                &[("switches", mode), ("date", date)],
            )
        };
        for op in [
            entry("Grillfest", &grill, "from: 2026-07-04, to: 2026-07-05"),
            entry("Betriebsurlaub", &vacation.to_uppercase(), "every: 08-01"),
            entry("Fasching", PARTY, "easter: -47"),
        ] {
            apply_op(&mut files, &ctx, &op).unwrap();
        }
        // Refused, changing nothing: no date, a date lox refuses, a stored
        // attribute, a mode the config does not have.
        for (op, says) in [
            (
                add(CAL_F, "CalendarEntry", None, &[], &[("switches", PARTY)]),
                "needs its date",
            ),
            (entry("X", PARTY, "every: 02-30"), "no day of the year"),
            (
                entry("X", PARTY, "from: 2008-12-30, to: 2009-01-02"),
                "2009-01-01",
            ),
            (
                add(CAL_F, "CalendarEntry", None, &[], &[("Month", "1")]),
                "not Month",
            ),
            (
                entry("X", "10000099-0000-0000-ffff504f94000001", "every: 01-01"),
                "no operating mode",
            ),
        ] {
            let was = files.clone();
            let e = apply_op(&mut files, &ctx, &op).unwrap_err();
            assert!(e.contains(says), "{e}");
            assert_eq!(files, was);
        }
        let text = files.render()[crate::periphery::FILE].clone();
        for line in [
            "vx_mode_1 = Mode(\"Grill\")\n",
            "vx_calendarentry_1 = CalendarEntry(\"Grillfest\", switches: vx_mode_1, from: 2026-07-04, to: 2026-07-05)\n",
            "vx_calendarentry_2 = CalendarEntry(\"Betriebsurlaub\", switches: vacation, every: 08-01)\n",
            "vx_calendarentry_3 = CalendarEntry(\"Fasching\", switches: \"Party\", easter: -47)\n",
        ] {
            assert!(text.contains(line), "{text}");
        }
        // Read back as written.
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().pages,
            files.pages
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        let at = |l: &Lockfile, out: &LoxoneDoc, s: &str| {
            let u = l.objects[s].uuid.clone();
            let el = find(&out.xml.root, &u).unwrap();
            let get = |a: &str| el.attr(a).map(String::from);
            (u, get("CalType"), get("Mode"), get("From"), get("To"))
        };
        let (fest, cal, mode, from, to) = at(&l, &out, "vx_calendarentry_1");
        assert_eq!(parent_of(&out, &fest).as_deref(), Some(CAL_F));
        assert_eq!(
            (
                cal.as_deref(),
                mode.as_deref(),
                from.as_deref(),
                to.as_deref()
            ),
            (Some("-2"), Some("3"), Some("6393"), Some("6394"))
        );
        assert_eq!(at(&l, &out, "vx_calendarentry_2").1.as_deref(), Some("1"));
        assert_eq!(at(&l, &out, "vx_calendarentry_3").1.as_deref(), Some("-1"));
        // What the config stores reads as vislox wrote it.
        let date = |u: &str, v: Option<&str>| Op::EntryDate {
            uuid: u.into(),
            value: v.map(Into::into),
        };
        let mode = |u: &str, v: Option<&str>| Op::EntryMode {
            uuid: u.into(),
            value: v.map(Into::into),
        };
        assert_eq!(
            current(&out, &date(&fest, None)).as_deref(),
            Some("from: 2026-07-04, to: 2026-07-05")
        );
        assert_eq!(
            current(&out, &mode(&fest, None)),
            Some(grill.to_lowercase())
        );
        let (fasching, ..) = at(&l, &out, "vx_calendarentry_3");
        assert_eq!(
            current(&out, &mode(&fasching, None)),
            Some(PARTY.to_lowercase())
        );
        let ops = files.ops_in(&lock_of(&l), Some(&out));
        assert!(
            ops.contains(&mode(&fest, Some(&grill.to_lowercase()))),
            "{ops:?}"
        );
        assert!(ops.contains(&date(&fest, Some("from: 2026-07-04, to: 2026-07-05"))));
        assert!(
            !ops.iter().any(|o| matches!(o, Op::Setting { .. })),
            "{ops:?}"
        );

        // Edited: another mode and another kind of date, undone as a whole.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let before = files.clone();
        let holiday = crate::sched_edit::builtin_uuid(1);
        let req = [date(&fest, Some("easter: 49")), mode(&fest, Some(&holiday))];
        let mut prev = Vec::new();
        for op in &req {
            prev.push(apply_op(&mut files, &ctx, op).unwrap().previous);
        }
        assert_eq!(
            prev[0],
            serde_json::json!("from: 2026-07-04, to: 2026-07-05")
        );
        assert_eq!(prev[1], serde_json::json!(grill.to_lowercase()));
        assert!(
            files.render()[crate::periphery::FILE]
                .contains("CalendarEntry(\"Grillfest\", switches: holiday, easter: 49)"),
            "{:?}",
            files.render()
        );
        let out2 = compile_files(&d, &files, &mut l).unwrap();
        let (_, cal, mode2, ..) = at(&l, &out2, "vx_calendarentry_1");
        assert_eq!((cal, mode2.as_deref()), (None, Some("1")));
        assert_eq!(
            current(&out2, &date(&fest, None)).as_deref(),
            Some("easter: 49")
        );
        let undo = undo_ops(&req, &prev, &[None, None]).unwrap();
        let lock = lock_of(&l);
        let ctx = ctx_of(&out2, &d, &lock, None, &names);
        for op in &undo {
            apply_op(&mut files, &ctx, op).unwrap();
        }
        assert_eq!(files.render(), before.render());

        // Refused: a bad date, an entry Loxone Config made, a mode an entry
        // switches on.
        let was = files.clone();
        let e = apply_op(
            &mut files,
            &ctx,
            &date(&fest, Some("from: 2026-07-05, to: 2026-07-04")),
        )
        .unwrap_err();
        assert!(e.contains("before"), "{e}");
        let e = apply_op(&mut files, &ctx, &date(HEILIGABEND, Some("every: 12-23"))).unwrap_err();
        assert!(e.contains("Loxone Config made"), "{e}");
        let e = apply_op(
            &mut files,
            &ctx,
            &Op::RemoveBlock {
                uuid: grill.clone(),
            },
        )
        .unwrap_err();
        assert!(e.contains("Grillfest"), "{e}");
        assert_eq!(files, was);
        let why = crate::calendar_edit::edit_refusals(&files, &ctx);
        assert!(why[&fest].is_null());
        assert!(why[HEILIGABEND].as_str().unwrap().contains("Loxone Config"));

        // Taken out and added again: with its mode by UUID and its date.
        let r = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: fest.clone() }).unwrap();
        let again: Op = serde_json::from_value(r.previous).unwrap();
        let Op::AddBlock { settings, .. } = &again else {
            panic!("{again:?}")
        };
        assert_eq!(settings["switches"], grill.to_lowercase());
        assert_eq!(settings["date"], "from: 2026-07-04, to: 2026-07-05");
        apply_op(&mut files, &ctx, &again).unwrap();
        compile_files(&d, &files, &mut l).unwrap();
    }

    #[test]
    fn a_mode_whose_slug_names_a_day_or_a_title_shared_is_spelled_as_lox_takes_it() {
        let d = doc("house.Loxone");
        let l = Lockfile::default();
        let lock = lock_of(&l);
        let modes = crate::sched_edit::Modes::new(&d, &lock);
        let sp = |u: &str| modes.spell(u).unwrap().to_string();
        assert_eq!(sp(&crate::sched_edit::builtin_uuid(1)), "holiday");
        assert_eq!(sp(&crate::sched_edit::builtin_uuid(11)), "heating");
        assert_eq!(sp(PARTY), "\"Party\"");
        // Read back by the same rules.
        for u in [crate::sched_edit::builtin_uuid(3), PARTY.to_lowercase()] {
            let v = modes.spell(&u).unwrap();
            assert_eq!(modes.uuid_of(&v), Some(u.to_lowercase()));
        }
    }

    #[test]
    fn a_mode_keeps_its_own_name_and_is_not_deleted_while_anything_names_it() {
        const G: &str = "10000091-0000-0000-ffff504f94000001";
        const K: &str = "10000092-0000-0000-ffff504f94000001";
        const E: &str = "10000093-0000-0000-ffff504f94000001";
        const W: &str = "10000094-0000-0000-ffff504f94000001";
        let p = format!("{}/tests/fixtures/house.Loxone", env!("CARGO_MANIFEST_DIR"));
        let house = std::fs::read_to_string(p).unwrap();
        // Kaputt has no `Mode=`: it numbers nothing. Grill is vislox's (-2);
        // Grillabend, Loxone Config's, switches it on by its number.
        let with = |entry: bool| {
            let mut s = house.replace(
                "Title=\"Betriebsmodi\">\n",
                &format!(
                    "Title=\"Betriebsmodi\">\n\t\t\t<C Type=\"Mode\" V=\"178\" U=\"{K}\" Title=\"Kaputt\"/>\n\
                     \t\t\t<C Type=\"Mode\" V=\"178\" U=\"{G}\" Title=\"Grill\" Mode=\"-2\"/>\n"
                ),
            );
            let mut entries = format!(
                "\t\t\t<C Type=\"CalendarEntry\" V=\"178\" U=\"{W}\" Title=\"Weg\" Month=\"6\" Day=\"2\" WeekDayInMonth=\"1\"/>\n"
            );
            if entry {
                entries.push_str(&format!(
                    "\t\t\t<C Type=\"CalendarEntry\" V=\"178\" U=\"{E}\" Title=\"Grillabend\" CalType=\"-2\" Month=\"6\" Day=\"1\" WeekDayInMonth=\"1\"/>\n"
                ));
            }
            s = s.replace(
                "Title=\"Betriebszeiten\">\n",
                &format!("Title=\"Betriebszeiten\">\n{entries}"),
            );
            LoxoneDoc::parse(s.as_bytes()).unwrap()
        };
        let (d, d2) = (with(true), with(false));
        let lock = Lock {
            slugs: [
                ("vx_mode_1".to_string(), G.to_string()),
                ("vx_calendarentry_1".to_string(), W.to_string()),
            ]
            .into(),
            ..Default::default()
        };
        let names = BTreeSet::new();
        let periph = "vx_mode_1 = Mode(\"Grill\")\n\
                      vx_calendarentry_1 = CalendarEntry(\"Weg\", switches: \"Fort\", every: 06-02)\n";
        let mut files = Files::from_texts(&BTreeMap::from([(
            crate::periphery::FILE.to_string(),
            periph.to_string(),
        )]))
        .unwrap();
        // A mode without a number never stands for the holiday mode.
        let n = crate::calendar_edit::Numbers::of(&d);
        let holiday = crate::sched_edit::builtin_uuid(1);
        assert_eq!(n.uuid(0), Some(holiday.clone()));
        let heiligabend = find(&d.xml.root, HEILIGABEND).unwrap();
        assert_eq!(
            crate::calendar_edit::mode_of(&n, heiligabend),
            Some(holiday.clone())
        );
        assert_eq!(n.uuid(-2), Some(G.to_lowercase()));

        // Loxone Config's entry names its number: not deleted.
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &Op::RemoveBlock { uuid: G.into() },
        )
        .unwrap_err();
        assert!(e.contains("Grillabend"), "{e}");
        // An entry deleted in the pending config (the base, deployed, still
        // has it) names nothing once that is deployed: the mode goes.
        let gone = ctx_of(&d2, &d, &lock, None, &names);
        assert!(
            apply_op(
                &mut files.clone(),
                &gone,
                &Op::RemoveBlock { uuid: G.into() }
            )
            .is_ok()
        );
        // Nor while a schedule set of vislox's names it by its slug.
        let ctx2 = ctx_of(&d2, &d2, &lock, None, &names);
        assert!(
            apply_op(
                &mut files.clone(),
                &ctx2,
                &Op::RemoveBlock { uuid: G.into() }
            )
            .is_ok()
        );
        let mut texts = files.render();
        texts.insert(
            "edits.loxl".into(),
            format!(
                "extern {s} = DayTimer(uuid: \"{UHR}\")\nschedule vx_a\n\tvx_mode_1 08:00-12:00\nend\n{s}.schedule = vx_a\n",
                s = slug(UHR)
            ),
        );
        let sched = Files::from_texts(&texts).unwrap();
        let e = apply_op(
            &mut sched.clone(),
            &ctx2,
            &Op::RemoveBlock { uuid: G.into() },
        )
        .unwrap_err();
        assert!(e.contains("schedule set `vx_a`"), "{e}");

        // Names: another mode's, compared case-blind, is refused; its own is kept.
        let was = files.clone();
        let e = apply_op(
            &mut files,
            &ctx,
            &add(MODE_F, "Mode", Some("party"), &[], &[]),
        )
        .unwrap_err();
        assert!(e.contains("has that name already"), "{e}");
        let title = |v: &str| Op::Title {
            uuid: G.into(),
            value: Some(v.into()),
        };
        let e = apply_op(&mut files, &ctx, &title("FEIERTAG")).unwrap_err();
        assert!(e.contains("Feiertag"), "{e}");
        assert_eq!(files, was);
        apply_op(&mut files, &ctx, &title("grill")).unwrap();

        // An entry added here keeps its mode and its date: neither is dropped,
        // so a remove's undo always adds it again.
        let was = files.clone();
        for op in [
            Op::EntryMode {
                uuid: W.into(),
                value: None,
            },
            Op::EntryDate {
                uuid: W.into(),
                value: None,
            },
        ] {
            let e = apply_op(&mut files, &ctx, &op).unwrap_err();
            assert!(e.contains("keeps its mode and its date"), "{e}");
        }
        assert_eq!(files, was);
        // An entry whose mode no longer resolves: undone exactly as written.
        let mode = |v: &str| Op::EntryMode {
            uuid: W.into(),
            value: Some(v.into()),
        };
        let r = apply_op(&mut files, &ctx, &mode(&holiday)).unwrap();
        assert_eq!(r.previous, serde_json::json!("\"Fort\""));
        apply_op(&mut files, &ctx, &mode("\"Fort\"")).unwrap();
        assert!(
            files.render()[crate::periphery::FILE].contains("switches: \"Fort\", every: 06-02")
        );
        let r = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: W.into() }).unwrap();
        let again: Op = serde_json::from_value(r.previous).unwrap();
        let Op::AddBlock { settings, .. } = &again else {
            panic!("{again:?}")
        };
        assert_eq!(settings["switches"], "\"Fort\"");
        apply_op(&mut files, &ctx, &again).unwrap();
        assert!(
            files.render()[crate::periphery::FILE].contains("switches: \"Fort\", every: 06-02")
        );
        // Text that is neither a UUID, a title nor a name is refused.
        assert!(apply_op(&mut files, &ctx, &mode("Fort;")).is_err());
    }

    // ── Automatic Designer rules (#87) ──────────────────────────────────

    fn rules_doc() -> LoxoneDoc {
        LoxoneDoc::parse(crate::rules::fixture::house().as_bytes()).unwrap()
    }

    fn rule_op(uuid: &str, v: Option<crate::rules::RuleVal>) -> Op {
        Op::Rule {
            uuid: uuid.into(),
            value: v,
        }
    }

    fn read_rule(doc: &LoxoneDoc, uuid: &str) -> Option<crate::rules::RuleVal> {
        match crate::rules::read(doc).remove(&uuid.to_ascii_lowercase())? {
            crate::rules::Read::Rule(v) => Some(crate::rule_edit::normalized(&v)),
            crate::rules::Read::Not(_) => None,
        }
    }

    #[test]
    fn a_rule_is_declared_compiled_by_lox_read_back_and_undone() {
        use crate::rules::fixture::{PARTY, PUMP, RULE_ODD, RULE_OFF};
        use crate::rules::{Line, RuleVal};
        let d = rules_doc();
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let want = RuleVal {
            active: true,
            lines: vec![
                Line::Weekday {
                    days: ["mon", "tue", "wed", "thu", "fri"]
                        .map(String::from)
                        .to_vec(),
                },
                Line::Time { at: "7:30".into() },
                Line::Or,
                Line::Mode {
                    mode: PARTY.to_uppercase(),
                    on: true,
                },
                Line::Then {
                    block: PUMP.into(),
                    command: "off".into(),
                },
                Line::Wait {
                    duration: "5s".into(),
                },
            ],
        };
        let a = apply_op(&mut files, &ctx, &rule_op(RULE_OFF, Some(want.clone()))).unwrap();
        assert_eq!(a.previous, serde_json::Value::Null);
        let text = files.claims.render();
        let (r, p) = (slug(RULE_OFF), slug(PUMP));
        for line in [
            format!("extern {r} = AutopilotRule(uuid: \"{RULE_OFF}\")\n"),
            format!("extern {p} = PushButton(uuid: \"{PUMP}\")\n"),
            format!(
                "rule {r}\n\tactive\n\twhen weekday weekdays\n\twhen time 07:30\n\tor\n\twhen mode \"Party\"\n\tthen {p} off\n\tthen wait 5s\nend\n"
            ),
        ] {
            assert!(text.contains(&line), "{text}");
        }
        assert_eq!(Claims::parse(&text).unwrap(), files.claims);
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(
            read_rule(&out, RULE_OFF),
            Some(crate::rule_edit::normalized(&want))
        );
        // The pending config is as the file states it.
        let lock = lock_of(&l);
        assert!(crate::rule_edit::missed(&files, &out, &lock, &[RULE_OFF.into()]).is_empty());
        // The other rules are untouched.
        assert_eq!(
            read_rule(&out, crate::rules::fixture::RULE_ON),
            read_rule(&d, crate::rules::fixture::RULE_ON)
        );
        // `GET /api/edits`: stated here, not deployed (the deployed config has
        // the rule otherwise), nothing to revert to but the rule as last written.
        let section =
            crate::rule_edit::edits_section(&files, &Files::default(), &lock, Some(&out), Some(&d));
        assert_eq!(section[0]["uuid"], RULE_OFF);
        assert_eq!(section[0]["pending"], true);
        assert_eq!(section[0]["deployed"], serde_json::Value::Null);
        let same = crate::rule_edit::edits_section(&files, &files, &lock, Some(&out), Some(&out));
        assert_eq!(same[0]["pending"], false);
        // Which rules vislox may state: not one Lox Logic cannot write.
        let e = crate::rule_edit::edit_section(&files, &ctx_of(&out, &d, &lock, None, &names));
        assert_eq!(e["rules"][RULE_OFF], serde_json::Value::Null);
        assert!(
            e["rules"][RULE_ODD]
                .as_str()
                .unwrap()
                .contains("cannot write")
        );
        assert_eq!(e["create"], serde_json::Value::Null);
        assert_eq!(
            e["offers"]["PushButton"]["commands"],
            serde_json::json!(["off", "on"])
        );

        // A second edit's previous is what the file stated, by UUID: its undo.
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let then = RuleVal {
            active: false,
            lines: vec![
                Line::Sun {
                    at: "sunset".into(),
                },
                Line::Then {
                    block: PUMP.into(),
                    command: "on".into(),
                },
            ],
        };
        let a = apply_op(&mut files, &ctx, &rule_op(RULE_OFF, Some(then.clone()))).unwrap();
        let prev: RuleVal = serde_json::from_value(a.previous.clone()).unwrap();
        assert_eq!(
            crate::rule_edit::normalized(&prev),
            crate::rule_edit::normalized(&want)
        );
        let undo = undo_ops(&[rule_op(RULE_OFF, Some(then))], &[a.previous], &[]).unwrap();
        assert_eq!(undo, vec![rule_op(RULE_OFF, Some(prev))]);

        // Refused, changing nothing: a rule Lox Logic cannot write, a block
        // the module manages, a `/` in a message, no action, an action before
        // a condition, a mode the config lacks, a block that is no rule.
        let mut managed = lock_of(&l);
        managed
            .slugs
            .insert("pumpe".into(), crate::rules::fixture::COVER.into());
        let mctx = ctx_of(&out, &d, &managed, None, &names);
        let at = Line::Time { at: "09:00".into() };
        let on = Line::Then {
            block: PUMP.into(),
            command: "on".into(),
        };
        let v = |lines: Vec<Line>| {
            Some(RuleVal {
                active: true,
                lines,
            })
        };
        for (op, c, says) in [
            (
                rule_op(RULE_ODD, v(vec![at.clone(), on.clone()])),
                &ctx,
                "cannot write this rule",
            ),
            (
                rule_op(
                    RULE_OFF,
                    v(vec![
                        at.clone(),
                        Line::Then {
                            block: crate::rules::fixture::COVER.into(),
                            command: "pulse".into(),
                        },
                    ]),
                ),
                &mctx,
                "managed by the module",
            ),
            (
                rule_op(
                    RULE_OFF,
                    v(vec![
                        at.clone(),
                        Line::Message {
                            severity: "info".into(),
                            subject: "a/b".into(),
                            text: "x".into(),
                            admins: false,
                        },
                    ]),
                ),
                &ctx,
                "cannot contain `/`",
            ),
            (
                rule_op(RULE_OFF, v(vec![at.clone()])),
                &ctx,
                "add an action",
            ),
            (
                rule_op(RULE_OFF, v(vec![on.clone(), at.clone()])),
                &ctx,
                "come before",
            ),
            (
                rule_op(
                    RULE_OFF,
                    v(vec![
                        Line::Mode {
                            mode: "40000099-0000-0990-ffff504f94112233".into(),
                            on: true,
                        },
                        on.clone(),
                    ]),
                ),
                &ctx,
                "no operating mode",
            ),
            (
                rule_op(PUMP, v(vec![at.clone(), on.clone()])),
                &ctx,
                "no Automatic Designer rule",
            ),
        ] {
            let was = files.clone();
            let e = apply_op(&mut files, c, &op).unwrap_err();
            assert!(e.contains(says), "{e}");
            assert_eq!(files, was);
        }
        // A mode the module names by its own slug is the module's to name.
        let mut moded = lock_of(&l);
        moded.externals.insert("party".into(), PARTY.into());
        let e = apply_op(
            &mut files.clone(),
            &ctx_of(&out, &d, &moded, None, &names),
            &rule_op(
                RULE_OFF,
                v(vec![
                    Line::Mode {
                        mode: PARTY.into(),
                        on: true,
                    },
                    on.clone(),
                ]),
            ),
        )
        .unwrap_err();
        assert!(e.contains("named `party` by the module"), "{e}");
        // A command the block does not offer is lox's to refuse, with the
        // commands it offers.
        let mut bad = files.clone();
        apply_op(
            &mut bad,
            &ctx,
            &rule_op(
                RULE_OFF,
                v(vec![
                    at.clone(),
                    Line::Then {
                        block: PUMP.into(),
                        command: "Mood 7".into(),
                    },
                ]),
            ),
        )
        .unwrap();
        let e = compile_files(&d, &bad, &mut l.clone()).unwrap_err();
        assert!(e.contains("Mood 7"), "{e}");

        // Dropping the declaration is always let through, and takes its
        // externs along.
        let a = apply_op(&mut files, &ctx, &rule_op(RULE_OFF, None)).unwrap();
        assert!(a.previous.is_object());
        assert_eq!(files.claims, Claims::default());
    }

    #[test]
    fn a_rule_is_created_by_name_renamed_deleted_and_brought_back() {
        use crate::rules::fixture::PUMP;
        use crate::rules::{Line, RuleVal};
        let d = rules_doc();
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = ctx_of(&d, &d, &lock, None, &names);
        let body = RuleVal {
            active: true,
            lines: vec![
                Line::Time { at: "12:30".into() },
                Line::Then {
                    block: PUMP.into(),
                    command: "on".into(),
                },
            ],
        };
        let new = |t: &str, v: Option<RuleVal>| Op::NewRule {
            title: t.into(),
            value: v,
            restores: None,
        };
        // Not a name another rule has (lox would take that one), nor empty,
        // nor in a config the app has put no rule into yet.
        for (t, says) in [("pumpe ein", "named \"pumpe ein\""), (" ", "needs a name")] {
            let e = apply_op(&mut files.clone(), &ctx, &new(t, None)).unwrap_err();
            assert!(e.contains(says), "{e}");
        }
        let bare = doc("house.Loxone");
        let e = apply_op(
            &mut files.clone(),
            &ctx_of(&bare, &bare, &lock, None, &names),
            &new("Neu", None),
        )
        .unwrap_err();
        assert!(e.contains("create one in the app"), "{e}");

        let a = apply_op(&mut files, &ctx, &new("Pumpe Mittag", Some(body.clone()))).unwrap();
        assert_eq!(a.added.as_deref(), Some("vx_rule_1"));
        let text = files.claims.render();
        assert!(
            text.contains(&format!(
                "rule vx_rule_1 \"Pumpe Mittag\"\n\tactive\n\twhen time 12:30\n\tthen {} on\nend\n",
                slug(PUMP)
            )),
            "{text}"
        );
        assert_eq!(Claims::parse(&text).unwrap(), files.claims);
        let out = compile_files(&d, &files, &mut l).unwrap();
        let uuid = l.rules["vx_rule_1"].clone();
        assert_eq!(
            find(&out.xml.root, &uuid).and_then(|e| e.attr("Title")),
            Some("Pumpe Mittag")
        );
        assert_eq!(
            read_rule(&out, &uuid),
            Some(crate::rule_edit::normalized(&body))
        );
        // Checked after the compile: the rule created as stated, and one the
        // config lacks named.
        let made = [new("Pumpe Mittag", Some(body.clone()))];
        let added = [Some("vx_rule_1".to_string())];
        let pinned = lock_of(&l);
        assert!(crate::rule_edit::missed_ops(&files, &out, &pinned, &made, &added).is_empty());
        assert_eq!(
            crate::rule_edit::missed_ops(&files, &d, &pinned, &made, &added),
            ["Pumpe Mittag"]
        );
        // The undo of creating it is removing it.
        assert_eq!(
            undo_ops(
                &[new("Pumpe Mittag", None)],
                &[a.previous],
                &[Some(uuid.clone())]
            )
            .unwrap(),
            vec![Op::RemoveBlock { uuid: uuid.clone() }]
        );

        // Renamed: the source owns its name.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let a = apply_op(
            &mut files,
            &ctx,
            &Op::Title {
                uuid: uuid.clone(),
                value: Some("Pumpe Mittags".into()),
            },
        )
        .unwrap();
        assert_eq!(a.previous, serde_json::json!("Pumpe Mittag"));
        let before = out.clone();
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(
            find(&out.xml.root, &uuid).and_then(|e| e.attr("Title")),
            Some("Pumpe Mittags")
        );
        // Only renamed: pending all the same, so it shows Revert and counts.
        let lock = lock_of(&l);
        let was = Files::from_texts(&files.render()).map(|mut f| {
            f.claims.9.claims.get_mut("vx_rule_1").unwrap().title = Some("Pumpe Mittag".into());
            f
        });
        let section = crate::rule_edit::edits_section(
            &files,
            &was.unwrap(),
            &lock,
            Some(&out),
            Some(&before),
        );
        assert_eq!(section[0]["pending"], true);
        assert_eq!(section[0]["deployed_title"], "Pumpe Mittag");
        // Deployed under that name, renamed, then renamed back (an undo, a
        // Revert) or in another case only: its own name is no other rule's.
        let ctx = ctx_of(&out, &out, &lock, None, &names);
        let mut renamed = files.clone();
        let title = |v: &str| Op::Title {
            uuid: uuid.clone(),
            value: Some(v.into()),
        };
        apply_op(&mut renamed, &ctx, &title("Pumpe Abends")).unwrap();
        apply_op(&mut renamed, &ctx, &title("Pumpe Mittags")).unwrap();
        apply_op(&mut renamed, &ctx, &title("pumpe mittags")).unwrap();
        // Another rule's name stays refused.
        assert!(
            apply_op(&mut renamed, &ctx, &title("Pumpe Ein"))
                .unwrap_err()
                .contains("already")
        );

        // Not deployed yet (the base lacks it): taken out, and its undo creates
        // it again.
        let lock = lock_of(&l);
        let ctx = ctx_of(&out, &d, &lock, None, &names);
        let mut gone = files.clone();
        let a = apply_op(&mut gone, &ctx, &Op::RemoveBlock { uuid: uuid.clone() }).unwrap();
        let back: Op = serde_json::from_value(a.previous).unwrap();
        assert!(
            matches!(&back, Op::NewRule { title, restores: Some(r), .. } if title == "Pumpe Mittags" && *r == uuid),
            "{back:?}"
        );
        assert!(gone.claims.9.is_empty());

        // Deployed (the base has it): `removed`, and back with its UUID.
        let base = out.clone();
        let ctx = ctx_of(&out, &base, &lock, None, &names);
        let a = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: uuid.clone() }).unwrap();
        let back: Op = serde_json::from_value(a.previous).unwrap();
        assert!(
            matches!(&back, Op::RestoreRule { slug, .. } if slug == "vx_rule_1"),
            "{back:?}"
        );
        assert!(files.claims.render().contains("removed vx_rule_1\n"));
        let out = compile_files(&base, &files, &mut l).unwrap();
        assert!(find_ci(&out.xml.root, &uuid).is_none());
        let lock = lock_of(&l);
        assert!(lock.tomb(&uuid).is_some());
        // A tombstone of another type is not a rule's to restore.
        let mut wrong = lock.clone();
        wrong
            .tombs
            .insert(uuid.clone(), ("vx_rule_1".into(), "Monoflop".into()));
        assert!(
            apply_op(
                &mut files.clone(),
                &ctx_of(&out, &base, &wrong, None, &names),
                &back
            )
            .unwrap_err()
            .contains("no deleted rule")
        );
        // Not under a name another rule has taken meanwhile.
        let taken = String::from_utf8(out.to_bytes())
            .unwrap()
            .replace("Title=\"Pumpe Ein\"", "Title=\"Pumpe Mittags\"");
        let taken = LoxoneDoc::parse(taken.as_bytes()).unwrap();
        let e = apply_op(
            &mut files.clone(),
            &ctx_of(&taken, &base, &lock, None, &names),
            &back,
        )
        .unwrap_err();
        assert!(e.contains("cannot come back under its name"), "{e}");
        let ctx = ctx_of(&out, &base, &lock, None, &names);
        let a = apply_op(&mut files, &ctx, &back).unwrap();
        assert_eq!(
            serde_json::from_value::<Op>(a.previous).unwrap(),
            Op::RemoveBlock { uuid: uuid.clone() }
        );
        let mut l = edit_lock(&l, &files.lock_edits);
        files.lock_edits = Default::default();
        let out = compile_files(&base, &files, &mut l).unwrap();
        assert!(find(&out.xml.root, &uuid).is_some());
        assert_eq!(l.rules["vx_rule_1"], uuid);
        assert_eq!(
            read_rule(&out, &uuid),
            Some(crate::rule_edit::normalized(&body))
        );
    }
}
