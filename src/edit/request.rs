//! What an edit request is: the [`Op`]s of `POST /api/edit` and the
//! revision they were made against ([`Request`]).

use super::*;

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
