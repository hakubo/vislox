//! The module's expressions and named signals (#82), and the blocks lox
//! made for each.
//!
//! lox desugars an expression (`x.Port <- a.Q and b.AQ >= 28`, `Port: …` in
//! a managed block's declaration, or a named signal `warm = …`) into `And`,
//! `Or`, `Not`, comparator and `Formula` blocks, labelled with their
//! sub-expressions (lox's `desugar.rs`). Their lockfile entries are
//! expression-owned and named `<sink>_<port key>__<op><n>` (an expression
//! into a port) or `<signal>__<op><n>` (a named signal, whose top block is
//! `<signal>` itself, hand-owned). So the blocks of one expression are the
//! lock's expression-owned entries under its prefix, and its top block the
//! last one the post-order walk names (one counter per operator, shared by
//! the expressions of one prefix, as lox counts).
//!
//! Each expression is read with lox's parser, at the top level of each file
//! (a template's body is #83's), and printed as lox prints it. What the
//! deployed revision had is read off the deployed config: the block that
//! feeds the sink (or the signal's top block by UUID), whose label is the
//! expression's text (lox writes `<=` as `≤`, `<` as `＜`, `%` as `％`).

use crate::edit::Lock;
use crate::managed::{Declarations, bindings, code, template_bodies};
use loxone_config::LoxoneDoc;
use loxone_config::logic::{
    ArgItem, Attr, BindingKind, CmpOp, Expr, Item, MatchSpec, Module, Operand, Value,
};
use loxone_config::port_names;
use loxone_config::xml::Element;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// The block types lox makes for an expression.
pub const TYPES: &[&str] = &[
    "And",
    "Or",
    "Not",
    "GreaterEqual",
    "Greater",
    "LessEqual",
    "Less",
    "Equal",
    "NotEqual",
    "Formula",
];

/// One expression of the module.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Expression {
    /// The prefix of its blocks' slugs: `<sink>_<port key>__` or
    /// `<signal>__`; a second expression on one prefix is `<prefix>#2`.
    pub id: String,
    /// `wire` (`x.Port <- …`), `argument` (`Port: …` in a declaration) or
    /// `signal` (`slug = …`).
    pub kind: &'static str,
    /// What it feeds, as written: `jal.AutoShade`; for a signal, its slug.
    pub sink: String,
    /// The input it feeds, `[object UUID, connector key]`, when known (not
    /// for a signal).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<[String; 2]>,
    /// The expression as lox prints it.
    pub text: String,
    /// A signal's label (`.title`), when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Shown as [`crate::managed::Source::file`]; 1-based line.
    pub file: String,
    pub line: Option<usize>,
    /// Written in vislox's files: edited there (the `expr` and `signal` ops).
    pub ours: bool,
    /// Its top block's UUID and type (the signal's block, or the one wired
    /// into the sink), when the lockfile has it.
    pub top: Option<String>,
    pub top_type: Option<&'static str>,
    /// The UUIDs of its blocks, the top one first.
    pub blocks: Vec<String>,
    /// What the deployed revision had: filled in by [`compare`].
    pub deployed: Option<Deployed>,
    /// The pending config's top label read back as Lox Logic, as
    /// `deployed.text` is: the two compare like with like.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    /// Whether that differs from what it is now (a new expression, a
    /// changed one, a new label).
    pub pending: bool,
}

/// An expression as the deployed config has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Deployed {
    /// The top block's label, as lox wrote it.
    pub label: String,
    /// The label read back as Lox Logic (`≤` as `<=`, …): the expression's
    /// text, unless a signal's `.title` labelled it.
    pub text: String,
    /// The UUIDs of its blocks then, the top one first.
    pub blocks: Vec<String>,
    /// The chain as compiled: each block's type, label and attributes and
    /// what feeds each of its connectors, its own blocks by structure (they
    /// get new UUIDs), the rest by UUID and connector.
    #[serde(skip)]
    pub shape: String,
}

/// The label lox gives an expression's block (`desugar::expr_label`).
#[cfg(test)]
pub fn label(text: &str) -> String {
    text.replace("<=", "≤")
        .replace('<', "＜")
        .replace('%', "％")
}

/// A label read back as Lox Logic.
pub fn unlabel(label: &str) -> String {
    label
        .replace('≤', "<=")
        .replace('＜', "<")
        .replace('％', "%")
}

/// The slugs lox gives the blocks of expression `e`, post-order, as its
/// desugar emits them: `<prefix><op><n>` with one counter per operator
/// shared by every expression of the prefix (`counters`), except a named
/// signal's top block, which takes the signal's slug and no number. The top
/// block's slug comes last.
fn slugs(
    e: &Expr,
    prefix: &str,
    signal: Option<&str>,
    counters: &mut BTreeMap<(String, &'static str), usize>,
) -> Vec<String> {
    fn node(
        e: &Expr,
        root: bool,
        prefix: &str,
        signal: Option<&str>,
        counters: &mut BTreeMap<(String, &'static str), usize>,
        out: &mut Vec<String>,
    ) {
        let op = match e {
            Expr::And(a, b) | Expr::Or(a, b) => {
                node(a, false, prefix, signal, counters, out);
                node(b, false, prefix, signal, counters, out);
                if matches!(e, Expr::And(..)) {
                    "and"
                } else {
                    "or"
                }
            }
            Expr::Not(x) => {
                node(x, false, prefix, signal, counters, out);
                "not"
            }
            Expr::Cmp { .. } if e.is_formula() => "f",
            Expr::Cmp { op, .. } => match op {
                CmpOp::Ge => "ge",
                CmpOp::Gt => "gt",
                CmpOp::Le => "le",
                CmpOp::Lt => "lt",
                CmpOp::Eq => "eq",
                CmpOp::Ne => "ne",
            },
            Expr::Arith { .. } => "f",
            // lox makes the Formula of the inner expression: never the root's.
            Expr::Atom(Operand::Arith(x)) => {
                node(x, false, prefix, signal, counters, out);
                return;
            }
            Expr::Atom(_) => return,
        };
        if root && let Some(s) = signal {
            out.push(s.to_string());
            return;
        }
        let n = counters.entry((prefix.to_string(), op)).or_insert(0);
        *n += 1;
        out.push(format!("{prefix}{op}{n}"));
    }
    let mut out = Vec::new();
    node(e, true, prefix, signal, counters, &mut out);
    out
}

/// One file parsed, with its lines.
struct Parsed<'a> {
    file: &'a str,
    lines: Vec<&'a str>,
    items: Vec<Item>,
}

/// The line (1-based) of `sink.port <- …` at the top level of `p`.
fn wire_line(p: &Parsed, sink: &str, port: &str) -> Option<usize> {
    let body = template_bodies(&p.lines);
    p.lines.iter().enumerate().find_map(|(n, l)| {
        if body.contains(&n) {
            return None;
        }
        let (lhs, _) = code(l).split_once("<-")?;
        let (s, q) = lhs.trim().split_once('.')?;
        (s.trim() == sink && q.trim() == port).then_some(n + 1)
    })
}

/// Every expression of the module's files (`files`: the path as shown, the
/// text), with its blocks in `lock`. `ours` tells a file of vislox's
/// directory. A file that does not parse is left out (`lox logic check`
/// says why, #65).
pub fn expressions<T: AsRef<str>>(
    files: &[(String, T)],
    lock: &Lock,
    ours: impl Fn(&str) -> bool,
) -> Vec<Expression> {
    let decls = Declarations::new(files);
    let parsed: Vec<Parsed> = files
        .iter()
        .filter_map(|(f, t)| {
            let t = t.as_ref();
            Some(Parsed {
                file: f.as_str(),
                lines: t.split('\n').collect(),
                items: Module::parse_fragment(t).ok()?.items,
            })
        })
        .collect();
    // Slug → (type, UUID when known), for externs and blocks.
    let mut objects: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    // A signal's label.
    let mut titles: BTreeMap<String, String> = BTreeMap::new();
    for p in &parsed {
        for item in &p.items {
            match item {
                Item::Extern(e) => {
                    let uuid = match &e.match_spec {
                        MatchSpec::Uuid(u) => Some(u.clone()),
                        _ => lock.externals.get(&e.slug).cloned(),
                    };
                    objects
                        .entry(e.slug.clone())
                        .or_insert((e.block_type.clone(), uuid));
                }
                Item::Block(b) => {
                    objects
                        .entry(b.slug.clone())
                        .or_insert((b.block_type.clone(), lock.slugs.get(&b.slug).cloned()));
                }
                Item::Attr(a) if a.attr == Attr::Title => {
                    let t = match &a.value {
                        Value::Str(s) => s.clone(),
                        v => v.to_string(),
                    };
                    titles.entry(a.slug.clone()).or_insert(t);
                }
                _ => {}
            }
        }
    }
    // Expression-owned entries of the lock, by slug.
    let owned: BTreeMap<&str, &str> = lock
        .entries
        .iter()
        .filter(|(_, e)| e.get("expr_owned").and_then(|x| x.as_bool()) == Some(true))
        .filter_map(|(s, e)| Some((s.as_str(), e.get("uuid")?.as_str()?)))
        .collect();
    let mut counters: BTreeMap<(String, &'static str), usize> = BTreeMap::new();
    // Expressions per prefix so far: a second one's id is `<prefix>#2`.
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = Vec::new();
    for p in &parsed {
        let mut add = |kind: &'static str,
                       sink: String,
                       to: Option<[String; 2]>,
                       prefix: String,
                       signal: Option<&str>,
                       e: &Expr,
                       line: Option<usize>| {
            // Each block is this expression's alone, by the slug lox gives it:
            // two expressions on one prefix number on.
            let mine = slugs(e, &prefix, signal, &mut counters);
            let uuid = |s: &String| -> Option<String> {
                if signal == Some(s.as_str()) {
                    return lock.uuid_of(s).map(String::from);
                }
                owned.get(s.as_str()).map(|u| u.to_string())
            };
            let top = mine.last().and_then(uuid);
            let mut blocks: Vec<String> = top.iter().cloned().collect();
            blocks.extend(mine.iter().rev().skip(1).filter_map(uuid));
            let n = seen.entry(prefix.clone()).or_insert(0);
            *n += 1;
            let id = if *n == 1 {
                prefix.clone()
            } else {
                format!("{prefix}#{n}")
            };
            out.push(Expression {
                id,
                kind,
                sink,
                to,
                text: e.to_string(),
                title: signal.and_then(|s| titles.get(s).cloned()),
                file: p.file.to_string(),
                line,
                ours: ours(p.file),
                top,
                top_type: e.top_block_type(),
                blocks,
                deployed: None,
                now: None,
                pending: false,
            });
        };
        for item in &p.items {
            match item {
                Item::ExprWire(w) => {
                    let (ty, uuid) = objects.get(&w.to.slug).cloned().unwrap_or_default();
                    let key = port_names::key_or_raw(&ty, &w.to.port).to_string();
                    add(
                        "wire",
                        w.to.to_string(),
                        uuid.map(|u| [u, key.clone()]),
                        format!("{}_{}__", w.to.slug, key.to_lowercase()),
                        None,
                        &w.expr,
                        wire_line(p, &w.to.slug, &w.to.port),
                    );
                }
                Item::Block(b) => {
                    let lines: BTreeMap<String, usize> = decls
                        .at(&b.slug, Some(p.file))
                        .map(|(_, l)| bindings(&p.lines, l - 1))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(k, n)| (k, n + 1))
                        .collect();
                    for a in &b.args {
                        let ArgItem::Binding(x) = a else { continue };
                        let BindingKind::Expr(e) = &x.kind else {
                            continue;
                        };
                        let key = port_names::key_or_raw(&b.block_type, &x.port).to_string();
                        add(
                            "argument",
                            format!("{}.{}", b.slug, x.port),
                            lock.slugs.get(&b.slug).map(|u| [u.clone(), key.clone()]),
                            format!("{}_{}__", b.slug, key.to_lowercase()),
                            None,
                            e,
                            lines.get(&x.port).copied(),
                        );
                    }
                }
                Item::Signal(x) => add(
                    "signal",
                    x.slug.clone(),
                    None,
                    format!("{}__", x.slug),
                    Some(&x.slug),
                    &x.expr,
                    decls.at(&x.slug, Some(p.file)).map(|(_, l)| l),
                ),
                _ => {}
            }
        }
    }
    out
}

/// The objects of a config by UUID (lowercase), and each connector's owner.
pub struct Index<'a> {
    objects: HashMap<String, &'a Element>,
    owner: HashMap<String, &'a Element>,
}

impl<'a> Index<'a> {
    pub fn of(doc: &'a LoxoneDoc) -> Index<'a> {
        fn walk<'a>(el: &'a Element, ix: &mut Index<'a>) {
            if el.name == "C"
                && let Some(u) = el.attr("U")
            {
                ix.objects.insert(u.to_ascii_lowercase(), el);
                for co in el.child_elements().filter(|c| c.name == "Co") {
                    if let Some(cu) = co.attr("U") {
                        ix.owner.insert(cu.to_ascii_lowercase(), el);
                    }
                }
            }
            for c in el.child_elements() {
                walk(c, ix);
            }
        }
        let mut ix = Index {
            objects: HashMap::new(),
            owner: HashMap::new(),
        };
        walk(&doc.xml.root, &mut ix);
        ix
    }

    fn get(&self, uuid: &str) -> Option<&'a Element> {
        self.objects.get(&uuid.to_ascii_lowercase()).copied()
    }

    /// The objects wired into the connectors of `el` (`key`: only that one).
    fn sources(&self, el: &Element, key: Option<&str>) -> Vec<&'a Element> {
        el.child_elements()
            .filter(|c| c.name == "Co")
            .filter(|c| key.is_none_or(|k| c.attr("K").is_some_and(|x| x.eq_ignore_ascii_case(k))))
            .flat_map(|c| c.child_elements().filter(|i| i.name == "In"))
            .filter_map(|i| {
                self.owner
                    .get(&i.attr("Input")?.to_ascii_lowercase())
                    .copied()
            })
            .collect()
    }
}

fn is_expr_block(el: &Element) -> bool {
    el.attr("Type").is_some_and(|t| TYPES.contains(&t))
}

/// What the deployed config `deployed` has for `e`: its top block (the one
/// wired into the sink, or the signal's by UUID) when lox made it for an
/// expression, its label, and the blocks of the chain then: those wired
/// into it, of the types lox makes, that are `e`'s now or gone from the
/// pending config (`pending`: the one shown, its index).
pub fn deployed(deployed: &Index, pending: Option<&Index>, e: &Expression) -> Option<Deployed> {
    let top = match (&e.to, &e.top) {
        (Some([uuid, key]), _) => {
            let sink = deployed.get(uuid)?;
            deployed
                .sources(sink, Some(key))
                .into_iter()
                .find(|s| is_expr_block(s))?
        }
        (None, Some(t)) => deployed.get(t).filter(|s| is_expr_block(s))?,
        _ => return None,
    };
    let now = |u: &str| e.blocks.iter().any(|b| b.eq_ignore_ascii_case(u));
    let mut blocks: Vec<String> = Vec::new();
    let mut todo = vec![top];
    while let Some(el) = todo.pop() {
        let Some(u) = el.attr("U") else { continue };
        if blocks.iter().any(|b| b.eq_ignore_ascii_case(u)) {
            continue;
        }
        blocks.push(u.to_string());
        for s in deployed.sources(el, None) {
            let Some(su) = s.attr("U") else { continue };
            let gone = pending.is_some_and(|p| p.get(su).is_none());
            if is_expr_block(s) && (now(su) || gone) {
                todo.push(s);
            }
        }
    }
    let label = top.attr_decoded("Title").unwrap_or_default().into_owned();
    let shape = shape(deployed, top, &blocks, 0);
    Some(Deployed {
        text: unlabel(&label),
        label,
        blocks,
        shape,
    })
}

/// [`Deployed::shape`] of the chain `chain` (UUIDs) from `el`.
fn shape(ix: &Index, el: &Element, chain: &[String], depth: usize) -> String {
    const SKIP: &[&str] = &["U", "V", "Px", "Py", "Px2", "Py2", "Cl", "Nio", "WF"];
    let mut out = String::from(el.attr("Type").unwrap_or_default());
    for a in &el.attrs {
        if !SKIP.contains(&a.name.as_str()) {
            out.push_str(&format!(" {}={:?}", a.name, a.value));
        }
    }
    let mut cos: Vec<&Element> = el.child_elements().filter(|c| c.name == "Co").collect();
    cos.sort_by_key(|c| c.attr("K").unwrap_or_default());
    for co in cos {
        out.push_str(&format!(" [{}", co.attr("K").unwrap_or_default()));
        if let Some(d) = co.attr("Def") {
            out.push_str(&format!("={d}"));
        }
        for i in co.child_elements().filter(|i| i.name == "In") {
            let input = i.attr("Input").unwrap_or_default().to_ascii_lowercase();
            let Some(src) = ix.owner.get(&input) else {
                out.push_str(&format!(" <-?{input}"));
                continue;
            };
            let su = src.attr("U").unwrap_or_default();
            if depth < 64 && chain.iter().any(|b| b.eq_ignore_ascii_case(su)) {
                out.push_str(&format!(" <-({})", shape(ix, src, chain, depth + 1)));
            } else {
                let key = src
                    .child_elements()
                    .find(|c| c.attr("U").is_some_and(|u| u.eq_ignore_ascii_case(&input)))
                    .and_then(|c| c.attr("K"))
                    .unwrap_or_default();
                out.push_str(&format!(" <-{}.{key}", su.to_ascii_lowercase()));
            }
        }
        out.push(']');
    }
    out
}

/// Fill in what the deployed revision had for each expression, and whether
/// the pending config has it otherwise: its top block's label (the whole
/// expression, as lox resolved it, or a signal's `.title`) or another top
/// block. Without a deployed config nothing is pending; without a pending
/// one (nothing compiled since) neither.
pub fn compare(
    exprs: &mut [Expression],
    deployed_doc: Option<&LoxoneDoc>,
    pending_doc: Option<&LoxoneDoc>,
) {
    let Some(d) = deployed_doc else { return };
    let dix = Index::of(d);
    let pix = pending_doc.map(Index::of);
    for e in exprs.iter_mut() {
        e.deployed = deployed(&dix, pix.as_ref(), e);
        let Some(p) = &pix else { continue };
        let now = deployed(p, None, e);
        // The chain as compiled, not only its top label: a titled signal's
        // label stays when its expression changes.
        e.pending = match (&e.deployed, &now) {
            (Some(was), Some(now)) => was.shape != now.shape,
            (None, None) => false,
            _ => true,
        };
        e.now = now.map(|n| n.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock(objects: &[(&str, &str, bool)]) -> Lock {
        let mut o = serde_json::Map::new();
        for (s, u, owned) in objects {
            o.insert(
                s.to_string(),
                serde_json::json!({ "uuid": u, "type": "And", "expr_owned": owned }),
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("logic.lock.json");
        std::fs::write(&p, serde_json::json!({ "objects": o }).to_string()).unwrap();
        Lock::read(&p)
    }

    #[test]
    fn expressions_are_found_with_their_blocks() {
        let text = "page \"EG\"\n\
                    extern jal = AutoJalousie(uuid: \"10000000-0000-0000-0000-000000000001\")\n\
                    let schwelle = 28\n\
                    jal.AutoShade <- sonne.Q and temp.AQ >= schwelle\n\
                    wp = And(\n\
                    \x20   In1: a.Q or b.Q,\n\
                    )\n\
                    warm = t.AQ / 1000 >= 1.5\n\
                    warm.title = \"Warm\"\n";
        let l = lock(&[
            ("jal_autoshade__ge1", "u-ge1", true),
            ("jal_autoshade__and1", "u-and1", true),
            ("wp", "u-wp", false),
            ("wp_i1__or1", "u-or1", true),
            ("warm", "u-warm", false),
            ("other__and1", "u-other", true),
        ]);
        let files = vec![("pages/eg.loxl".to_string(), text)];
        let got = expressions(&files, &l, |f| f.starts_with("pages/vislox/"));
        assert_eq!(got.len(), 3, "{got:#?}");
        let w = &got[0];
        assert_eq!(
            (w.kind, w.sink.as_str(), w.text.as_str(), w.line),
            (
                "wire",
                "jal.AutoShade",
                "sonne.Q and temp.AQ >= schwelle",
                Some(4)
            )
        );
        assert_eq!(w.id, "jal_autoshade__");
        assert_eq!(
            w.to,
            Some([
                "10000000-0000-0000-0000-000000000001".into(),
                "AutoShade".into()
            ])
        );
        assert_eq!(w.top.as_deref(), Some("u-and1"));
        assert_eq!(w.blocks, ["u-and1", "u-ge1"]);
        assert!(!w.ours);
        let a = &got[1];
        assert_eq!(
            (a.kind, a.sink.as_str(), a.line),
            ("argument", "wp.In1", Some(6))
        );
        assert_eq!(a.to, Some(["u-wp".into(), "I1".into()]));
        assert_eq!(a.blocks, ["u-or1"]);
        let s = &got[2];
        assert_eq!(
            (s.kind, s.sink.as_str(), s.line),
            ("signal", "warm", Some(8))
        );
        assert_eq!(s.title.as_deref(), Some("Warm"));
        assert_eq!(s.top_type, Some("Formula"));
        assert_eq!(s.blocks, ["u-warm"]);
    }

    #[test]
    fn two_expressions_on_one_prefix_count_on() {
        let text = "x.I1 <- a.Q and b.Q\nx.I1 <- c.Q and d.Q\n";
        let l = lock(&[("x_i1__and1", "u1", true), ("x_i1__and2", "u2", true)]);
        let files = vec![("m.loxl".to_string(), text)];
        let got = expressions(&files, &l, |_| false);
        assert_eq!(got[0].top.as_deref(), Some("u1"));
        assert_eq!(got[1].top.as_deref(), Some("u2"));
        // Each block is one expression's: no overlap, and distinct ids.
        assert_eq!(got[0].blocks, ["u1"]);
        assert_eq!(got[1].blocks, ["u2"]);
        assert_eq!(
            (got[0].id.as_str(), got[1].id.as_str()),
            ("x_i1__", "x_i1__#2")
        );
        // Deeper chains number on across both: `a and b or c` takes and1, or1;
        // the second's `d and e or f` and2, or2.
        let text = "x.I1 <- a.Q and b.Q or c.Q\nx.I1 <- d.Q and e.Q or f.Q\n";
        let l = lock(&[
            ("x_i1__and1", "a1", true),
            ("x_i1__or1", "o1", true),
            ("x_i1__and2", "a2", true),
            ("x_i1__or2", "o2", true),
        ]);
        let files = vec![("m.loxl".to_string(), text)];
        let got = expressions(&files, &l, |_| false);
        assert_eq!(got[0].blocks, ["o1", "a1"]);
        assert_eq!(got[1].blocks, ["o2", "a2"]);
    }

    #[test]
    fn labels_read_back() {
        assert_eq!(
            label("a.AQ <= 5 and b.AQ < 70%"),
            "a.AQ ≤ 5 and b.AQ ＜ 70％"
        );
        assert_eq!(
            unlabel("a.AQ ≤ 5 and b.AQ ＜ 70％"),
            "a.AQ <= 5 and b.AQ < 70%"
        );
    }

    #[test]
    fn the_deployed_chain_is_read_off_the_config() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<ControlList>
<C Type="LoxLIVE" U="0">
<C Type="Page" U="p">
<C Type="AutoJalousie" U="sink" Title="Jal"><Co K="AutoShade" U="sink-1"><In Input="and-q"/></Co></C>
<C Type="And" U="and" Title="sonne.Q and temp.AQ &gt;= 28"><Co K="I1" U="and-1"/><Co K="I2" U="and-2"><In Input="ge-q"/></Co><Co K="Q" U="and-q"/></C>
<C Type="GreaterEqual" U="ge" Title="temp.AQ &gt;= 28"><Co K="Q" U="ge-q"/></C>
</C>
</C>
</ControlList>"#;
        let doc = LoxoneDoc::parse(xml.as_bytes()).unwrap();
        let ix = Index::of(&doc);
        let mut e = Expression {
            id: "jal_autoshade__".into(),
            kind: "wire",
            sink: "jal.AutoShade".into(),
            to: Some(["SINK".into(), "AutoShade".into()]),
            text: "sonne.Q and temp.AQ >= 30".into(),
            title: None,
            file: "m.loxl".into(),
            line: Some(1),
            ours: false,
            top: Some("and".into()),
            top_type: Some("And"),
            blocks: vec!["and".into()],
            deployed: None,
            now: None,
            pending: false,
        };
        let d = deployed(&ix, None, &e).unwrap();
        assert_eq!(d.text, "sonne.Q and temp.AQ >= 28");
        // Not `e`'s now, and not known gone: left out.
        assert_eq!(d.blocks, ["and"]);
        e.blocks.push("GE".into());
        assert_eq!(deployed(&ix, None, &e).unwrap().blocks, ["and", "ge"]);
        // Nothing compiled since: nothing pending.
        compare(std::slice::from_mut(&mut e), Some(&doc), None);
        assert!(!e.pending && e.deployed.is_some());
        compare(std::slice::from_mut(&mut e), Some(&doc), Some(&doc));
        assert!(!e.pending);
    }
}
