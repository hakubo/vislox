//! vislox's files in the module, read and written: the claims file
//! ([`Claims`], [`FILE`]) and the files of the blocks vislox adds
//! ([`Files`]). Only what vislox writes is read back.

use super::*;

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
    pub(super) fn render(&self) -> String {
        format!(
            "{}.{} <- {}.{}",
            self.to.0, self.to.1, self.from.0, self.from.1
        )
    }

    pub(super) fn touches(&self, slug: &str) -> bool {
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
    pub(super) fn is_empty(&self) -> bool {
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
                for n in blocks::value_refs(b) {
                    out.push((n.to_string(), name.clone()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::tests::DIV;

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
}
