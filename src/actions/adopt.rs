//! The scratch copy an adoption (#72) is worked out on: `lox logic adopt`
//! runs on copies of the module and lockfile, never on the project's, and
//! [`crate::adopt::extract`] turns what it wrote into vislox's files. The
//! edit then writes those and compiles, like any other edit.

use super::*;
use crate::adopt::{self, Adoption};
use std::collections::{BTreeMap, BTreeSet};

/// What the scratch copy made of a request's `adopt` and `adopt_page` ops.
#[derive(Default)]
pub(super) struct Prepared {
    /// By UUID.
    pub adoptions: BTreeMap<String, Adoption>,
    /// For each `adopt_page`, the blocks it adopts, by page UUID.
    pub pages: BTreeMap<String, Vec<String>>,
    /// Blocks an `adopt_page` leaves to Loxone Config: `{ uuid, title, why }`.
    pub skipped: Vec<Value>,
}

/// A block to adopt on the scratch copy.
struct Cand {
    uuid: String,
    title: String,
    slug: String,
    /// The `adopt_page` it comes from; `None`: an `adopt` asked for it, so a
    /// refusal refuses the request.
    page: Option<String>,
}

/// The files of a module directory (not hidden ones, as lox skips them), by
/// their paths relative to it, with `/`: the `.loxl` texts, and the bytes of
/// any other file (a program a `Code: file(…)` names).
#[derive(Clone, Default, PartialEq)]
struct Tree {
    loxl: BTreeMap<String, String>,
    other: BTreeMap<String, Vec<u8>>,
}

impl Tree {
    fn read(dir: &Path) -> Tree {
        fn walk(root: &Path, d: &Path, out: &mut Tree) {
            let Ok(entries) = std::fs::read_dir(d) else {
                return;
            };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let p = e.path();
                if p.is_dir() {
                    walk(root, &p, out);
                    continue;
                }
                let rel = p
                    .strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel.ends_with(".loxl") {
                    if let Ok(t) = std::fs::read_to_string(&p) {
                        out.loxl.insert(rel, t);
                    }
                } else if let Ok(b) = std::fs::read(&p) {
                    out.other.insert(rel, b);
                }
            }
        }
        let mut out = Tree::default();
        walk(dir, dir, &mut out);
        out
    }

    /// Make `dir` hold exactly this tree.
    fn write(&self, dir: &Path) -> Result<(), String> {
        let now = Tree::read(dir);
        for rel in now.loxl.keys().chain(now.other.keys()) {
            if !self.loxl.contains_key(rel) && !self.other.contains_key(rel) {
                std::fs::remove_file(dir.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
            }
        }
        let files = self
            .loxl
            .iter()
            .map(|(r, t)| (r, t.as_bytes()))
            .chain(self.other.iter().map(|(r, b)| (r, b.as_slice())));
        for (rel, b) in files {
            let p = dir.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{rel}: {e}"))?;
            }
            std::fs::write(&p, b).map_err(|e| format!("{rel}: {e}"))?;
        }
        Ok(())
    }

    /// A file other than a `.loxl` one that is added, changed or gone in
    /// `after`: lox wrote what vislox does not.
    fn other_changed(&self, after: &Tree) -> Option<String> {
        self.other
            .keys()
            .chain(after.other.keys())
            .find(|rel| self.other.get(*rel) != after.other.get(*rel))
            .cloned()
    }
}

/// lox's message, without its `error: ` prefix.
fn lox_says(stdout: &str, stderr: &str) -> String {
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    last_lines(text, 6)
        .lines()
        .map(|l| l.strip_prefix("error: ").unwrap_or(l))
        .map(|l| l.strip_prefix("compile error: ").unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Why vislox does not hand `u` to `lox logic adopt`, if it does not: not in
/// the base (lox adopts from it), managed already, not on a logic page, a
/// type lox has no table for, claimed by vislox's edits, or an extern of the
/// module's own files (lox would rewrite them).
pub(super) fn why_not(
    u: &str,
    base: &adopt::Index,
    doc: &adopt::Index,
    lock: &edit::Lock,
    files: &edit::Files,
) -> Option<String> {
    let name = if doc.get(u).is_some() {
        doc.name(u)
    } else {
        base.name(u)
    };
    let Some(el) = base.get(u) else {
        return Some(format!(
            "{name} is not in the project's base config, which lox adopts from"
        ));
    };
    if lock.slug_of(u).is_some() {
        return Some(format!("{name} is managed by the module already"));
    }
    match (&el.page, doc.get(u).and_then(|e| e.page.as_ref())) {
        (Some(a), Some(b)) if a == b => {}
        (Some(_), Some(_)) => {
            return Some(format!(
                "{name} is on another page in the pending config than in the base"
            ));
        }
        _ => {
            return Some(format!(
                "{name} is not on a logic page: vislox adopts only the blocks of a page"
            ));
        }
    }
    if loxone_config::connectors::builtin(&el.ty).is_none() {
        return Some(format!("lox cannot manage blocks of type {}", el.ty));
    }
    let s = edit::slug(u);
    if files.claims.0.get(u).is_some_and(|c| c.claims())
        || files.claims.1.iter().any(|w| w.to.0 == s || w.from.0 == s)
        || files.claims.set_claimed(&s)
    {
        return Some(format!(
            "{name}: vislox's edits claim values, wires, switching times, moods or members on \
             it ({}); revert them first, then adopt it",
            edit::FILE
        ));
    }
    if let Some((x, _)) = lock
        .externals
        .iter()
        .find(|(x, v)| v.as_str() == u && !x.starts_with(crate::blocks::SLUG_PREFIX))
    {
        return Some(format!(
            "{name} is the module's extern `{x}`: lox adopts it only under that name and \
             moves the module's own lines into it, and vislox writes only its own files. \
             Adopt it with lox (`lox logic adopt … --as {x}`)"
        ));
    }
    None
}

/// `cands` with each block after the blocks among them that feed it (in
/// the base); a loop keeps the order it had.
fn order(cands: &mut Vec<Cand>, base: &loxone_config::LoxoneDoc, index: &adopt::Index) {
    let mut owner: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, c) in cands.iter().enumerate() {
        for co in index.get(&c.uuid).into_iter().flat_map(|e| &e.ports) {
            owner.insert(co, i);
        }
    }
    let mut feeds: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); cands.len()];
    for w in base.wires() {
        if let (Some(&a), Some(&b)) = (
            owner.get(w.from_port.as_str()),
            owner.get(w.to_port.as_str()),
        ) && a != b
        {
            feeds[b].insert(a);
        }
    }
    let mut done: BTreeSet<usize> = BTreeSet::new();
    let mut order: Vec<usize> = Vec::with_capacity(cands.len());
    let mut left: Vec<usize> = (0..cands.len()).collect();
    while !left.is_empty() {
        let next = left
            .iter()
            .position(|i| feeds[*i].is_subset(&done))
            .unwrap_or(0);
        let i = left.remove(next);
        done.insert(i);
        order.push(i);
    }
    let mut taken: Vec<Option<Cand>> = std::mem::take(cands).into_iter().map(Some).collect();
    *cands = order.into_iter().filter_map(|i| taken[i].take()).collect();
}

impl Actions {
    /// Run `lox logic adopt` for each block the request's ops adopt, in
    /// order, on a scratch copy of the module and lockfile, and work out what
    /// vislox writes for each. Changes nothing in the project. An `adopt` lox
    /// refuses (or warns about) refuses the request with lox's message; an
    /// `adopt_page` skips that block and says why.
    pub(super) async fn adopt_prepare(
        &self,
        p: &loxone_config::Project,
        files: &edit::Files,
        lock: &edit::Lock,
        base: &loxone_config::LoxoneDoc,
        doc: &loxone_config::LoxoneDoc,
        ops: &[edit::Op],
    ) -> Result<Prepared, Refusal> {
        use axum::http::StatusCode;
        let bad = |e: String| (StatusCode::UNPROCESSABLE_ENTITY, e);
        let fail = |e: String| (StatusCode::INTERNAL_SERVER_ERROR, e);
        let (names, lets) = edit::module_decls(&p.module);
        let mut chosen: BTreeSet<String> = BTreeSet::new();
        let taken = |s: &str, chosen: &BTreeSet<String>| {
            names.contains(s)
                || lock.slugs.contains_key(s)
                || lock.tombstones.contains(s)
                || lock.externals.contains_key(s)
                || files.names(s)
                || files.claims.0.keys().any(|u| edit::slug(u) == s)
                || chosen.contains(s)
        };
        let (base_ix, doc_ix) = (adopt::Index::of(base), adopt::Index::of(doc));
        let title_of = |u: &str| {
            if doc_ix.get(u).is_some() {
                doc_ix.name(u)
            } else {
                base_ix.name(u)
            }
        };
        let why_not = |u: &str| why_not(u, &base_ix, &doc_ix, lock, files);
        let mut cands: Vec<Cand> = Vec::new();
        let mut pages: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut skipped: Vec<Value> = Vec::new();
        for op in ops {
            match op {
                edit::Op::Adopt { blocks } => {
                    for b in blocks {
                        if cands.iter().any(|c| c.uuid == b.uuid) {
                            return Err(bad(format!("{} is adopted twice", title_of(&b.uuid))));
                        }
                        if let Some(why) = why_not(&b.uuid) {
                            return Err(bad(why));
                        }
                        let ty = base_ix.get(&b.uuid).expect("checked").ty.clone();
                        let slug = match &b.slug {
                            Some(s) => {
                                adopt::check_slug(s).map_err(bad)?;
                                if taken(s, &chosen) {
                                    return Err(bad(format!(
                                        "`{s}` is taken in the module or its lockfile; pick another slug"
                                    )));
                                }
                                s.clone()
                            }
                            None => {
                                adopt::default_slug(&title_of(&b.uuid), &ty, &|s| taken(s, &chosen))
                            }
                        };
                        chosen.insert(slug.clone());
                        cands.push(Cand {
                            uuid: b.uuid.clone(),
                            title: title_of(&b.uuid),
                            slug,
                            page: None,
                        });
                    }
                }
                edit::Op::AdoptPage { page } => {
                    if base_ix.get(page).is_none_or(|e| e.ty != "Page") {
                        return Err(bad(format!("no page {page} in the project's base config")));
                    }
                    pages.entry(page.clone()).or_default();
                    for (u, ty, title) in adopt::on_page(doc, page) {
                        if lock.slug_of(&u).is_some() || cands.iter().any(|c| c.uuid == u) {
                            continue;
                        }
                        if let Some(why) = why_not(&u) {
                            skipped.push(json!({ "uuid": u, "title": title, "why": why }));
                            continue;
                        }
                        let slug = adopt::default_slug(&title, &ty, &|s| taken(s, &chosen));
                        chosen.insert(slug.clone());
                        cands.push(Cand {
                            uuid: u,
                            title,
                            slug,
                            page: Some(page.clone()),
                        });
                    }
                }
                _ => {}
            }
        }
        // Sources before the blocks they feed: lox adopts a block wired into
        // a managed one only when that one's declaration binds the wire, and
        // it binds it when it adopts the sink after the source.
        order(&mut cands, base, &base_ix);
        if cands.is_empty() {
            let why: Vec<String> = skipped
                .iter()
                .filter_map(|s| s["why"].as_str().map(String::from))
                .take(5)
                .collect();
            return Err(bad(if why.is_empty() {
                "There is no block here the module does not manage yet.".to_string()
            } else {
                format!("lox can adopt no block of this page:\n{}", why.join("\n"))
            }));
        }
        if !p.lock.exists() {
            return Err((
                StatusCode::CONFLICT,
                "The project has no lockfile yet: compile the module once, then adopt".into(),
            ));
        }
        // The scratch copy: the module and lock, removed with `_gone`.
        let d = removals::scratch_dir("vislox-adopt").map_err(fail)?;
        let _gone = RemoveDir(d.clone());
        let (module, lock_path) = (d.join("module"), d.join("lock.json"));
        let original = Tree::read(&p.module);
        original
            .write(&module)
            .map_err(|e| fail(format!("cannot copy the module: {e}")))?;
        let lock0 =
            std::fs::read(&p.lock).map_err(|e| fail(format!("cannot read the lock: {e}")))?;
        std::fs::write(&lock_path, &lock0)
            .map_err(|e| fail(format!("cannot copy the lock: {e}")))?;
        let parse = |b: &[u8]| -> Result<Value, Refusal> {
            serde_json::from_slice(b).map_err(|e| fail(format!("cannot read the lock: {e}")))
        };
        let l0 = parse(&lock0)?;
        let base_path = p.base.to_string_lossy().into_owned();
        let (m, l) = (
            module.to_string_lossy().into_owned(),
            lock_path.to_string_lossy().into_owned(),
        );
        let mut picked: BTreeMap<String, String> = BTreeMap::new();
        // The copy before each run: what a refused run puts back.
        let (mut tree, mut lock_was, mut lock_now) = (original.clone(), lock0, l0.clone());
        for c in &cands {
            // A block pinned as an extern (by an earlier adoption of this
            // request, or vislox's extern of a block adopted before) is
            // adopted under that name: lox moves the lines on the extern into
            // its declaration. vislox's slug replaces it after.
            let as_slug = lock_now["externals"]
                .as_object()
                .and_then(|x| {
                    x.iter()
                        .find(|(_, v)| v["uuid"].as_str() == Some(c.uuid.as_str()))
                        .map(|(k, _)| k.clone())
                })
                .unwrap_or_else(|| c.slug.clone());
            let args = [
                "logic", "adopt", &base_path, "--uuid", &c.uuid, "--as", &as_slug, "--module", &m,
                "--lock", &l,
            ];
            // From the project, as every lox command vislox runs; it writes only
            // the copies its arguments name.
            let res = run_out(&self.s.lox, &args, &p.root, COMPILE_TIMEOUT).await;
            let after = Tree::read(&module);
            let refused = match &res {
                Err(e) => Some(e.clone()),
                Ok((false, o, e)) => Some(lox_says(o, e)),
                Ok((true, _, e)) => {
                    let warnings: Vec<&str> = e
                        .lines()
                        .filter(|l| l.trim_start().starts_with("warning"))
                        .collect();
                    if !warnings.is_empty() {
                        Some(format!(
                            "lox warns, so it stays Loxone Config's:\n{}",
                            warnings.join("\n")
                        ))
                    } else {
                        // Anything but the module's `.loxl` files is not vislox's to write.
                        tree.other_changed(&after).map(|rel| {
                            format!(
                                "lox writes part of it into a file of its own ({rel}); vislox \
                                 writes only .loxl declarations"
                            )
                        })
                    }
                }
            };
            match refused {
                None => {
                    picked.insert(c.uuid.clone(), c.slug.clone());
                    if let Some(pg) = &c.page {
                        pages.entry(pg.clone()).or_default().push(c.uuid.clone());
                    }
                    lock_was = std::fs::read(&lock_path).map_err(|e| fail(e.to_string()))?;
                    lock_now = parse(&lock_was)?;
                    tree = after;
                }
                Some(why) => {
                    // lox writes nothing when it refuses; after a warning
                    // or a file vislox does not write, the copy goes back.
                    if after != tree {
                        tree.write(&module).map_err(fail)?;
                    }
                    std::fs::write(&lock_path, &lock_was).map_err(|e| fail(e.to_string()))?;
                    match c.page {
                        None => return Err(bad(format!("lox cannot adopt {}: {why}", c.title))),
                        Some(_) => {
                            skipped.push(json!({ "uuid": c.uuid, "title": c.title, "why": why }))
                        }
                    }
                }
            }
        }
        if picked.is_empty() {
            let why: Vec<String> = skipped
                .iter()
                .filter_map(|s| s["why"].as_str().map(|w| w.to_string()))
                .take(5)
                .collect();
            return Err(bad(format!(
                "lox can adopt no block of this page:\n{}",
                why.join("\n")
            )));
        }
        let own = |s: &str| files.block(s).is_some();
        let found = adopt::extract(
            &original.loxl,
            &tree.loxl,
            (&l0, &lock_now),
            &picked,
            &own,
            &lets,
            base,
        )
        .map_err(bad)?;
        Ok(Prepared {
            adoptions: found.into_iter().map(|a| (a.uuid.clone(), a)).collect(),
            pages,
            skipped,
        })
    }
}
