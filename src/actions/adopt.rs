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

/// Every file under `dir` (not hidden ones, as lox skips them), by its path
/// relative to `dir`, with `/`.
fn read_tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, d: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
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
            } else if let Ok(b) = std::fs::read(&p) {
                let rel = p.strip_prefix(root).unwrap_or(&p);
                out.insert(rel.to_string_lossy().replace('\\', "/"), b);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Put `dir` back to `tree` (as [`read_tree`] read it).
fn write_tree(dir: &Path, tree: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    for (rel, now) in read_tree(dir) {
        if tree.get(&rel) != Some(&now) && !tree.contains_key(&rel) {
            std::fs::remove_file(dir.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
        }
    }
    for (rel, b) in tree {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{rel}: {e}"))?;
        }
        std::fs::write(&p, b).map_err(|e| format!("{rel}: {e}"))?;
    }
    Ok(())
}

fn loxl_texts(tree: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, String> {
    tree.iter()
        .filter(|(p, _)| p.ends_with(".loxl"))
        .map(|(p, b)| (p.clone(), String::from_utf8_lossy(b).into_owned()))
        .collect()
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
    base: &loxone_config::LoxoneDoc,
    doc: &loxone_config::LoxoneDoc,
    lock: &edit::Lock,
    files: &edit::Files,
) -> Option<String> {
    let title_of = |u: &str| {
        edit::find(&doc.xml.root, u)
            .or_else(|| edit::find(&base.xml.root, u))
            .and_then(|e| e.attr_decoded("Title").map(|t| t.into_owned()))
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| u.to_string())
    };
    let name = title_of(u);
    let Some(el) = edit::find(&base.xml.root, u) else {
        return Some(format!(
            "{name} is not in the project's base config, which lox adopts from"
        ));
    };
    let ty = el.attr("Type").unwrap_or_default();
    if lock.slug_of(u).is_some() {
        return Some(format!("{name} is managed by the module already"));
    }
    match (adopt::page_of(base, u), adopt::page_of(doc, u)) {
        (Some((a, _)), Some((b, _))) if a == b => {}
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
    if loxone_config::connectors::builtin(ty).is_none() {
        return Some(format!("lox cannot manage blocks of type {ty}"));
    }
    if files.claims.0.get(u).is_some_and(|c| c.claims())
        || files.claims.1.iter().any(|w| {
            let s = edit::slug(u);
            w.to.0 == s || w.from.0 == s
        })
    {
        return Some(format!(
            "{name}: vislox's edits claim values or wires on it ({}); revert them first, \
                 then adopt it",
            edit::FILE
        ));
    }
    if files.claims.0.get(u).is_some_and(|c| c.pinned) {
        return Some(format!(
            "{name} is wired to a block adopted here: lox would rewrite that block's lines. \
                 Adopt them together (Manage every block on this page), or hand that block back first"
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
fn order(cands: &mut Vec<Cand>, base: &loxone_config::LoxoneDoc) {
    let mut owner: BTreeMap<String, usize> = BTreeMap::new();
    for (i, c) in cands.iter().enumerate() {
        if let Some(el) = edit::find(&base.xml.root, &c.uuid) {
            for co in el.child_elements().filter(|x| x.name == "Co") {
                if let Some(u) = co.attr("U") {
                    owner.insert(u.to_string(), i);
                }
            }
        }
    }
    let mut feeds: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); cands.len()];
    for w in base.wires() {
        if let (Some(&a), Some(&b)) = (owner.get(&w.from_port), owner.get(&w.to_port))
            && a != b
        {
            feeds[b].insert(a);
        }
    }
    let mut done: Vec<usize> = Vec::new();
    let mut left: Vec<usize> = (0..cands.len()).collect();
    while !left.is_empty() {
        let next = left
            .iter()
            .position(|i| feeds[*i].iter().all(|f| done.contains(f)))
            .unwrap_or(0);
        done.push(left.remove(next));
    }
    let mut taken: Vec<Option<Cand>> = std::mem::take(cands).into_iter().map(Some).collect();
    *cands = done.into_iter().filter_map(|i| taken[i].take()).collect();
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
        let names = edit::module_names(&p.module);
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
        let title_of = |u: &str| {
            edit::find(&doc.xml.root, u)
                .or_else(|| edit::find(&base.xml.root, u))
                .and_then(|e| e.attr_decoded("Title").map(|t| t.into_owned()))
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| u.to_string())
        };
        let why_not = |u: &str| why_not(u, base, doc, lock, files);
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
                        let el = edit::find(&base.xml.root, &b.uuid).expect("checked");
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
                            None => adopt::default_slug(
                                &title_of(&b.uuid),
                                el.attr("Type").unwrap_or_default(),
                                &|s| taken(s, &chosen),
                            ),
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
                    if edit::find(&base.xml.root, page)
                        .is_none_or(|e| e.attr("Type") != Some("Page"))
                    {
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
        order(&mut cands, base);
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
        let original = read_tree(&p.module);
        write_tree(&module, &original).map_err(|e| fail(format!("cannot copy the module: {e}")))?;
        std::fs::copy(&p.lock, &lock_path)
            .map_err(|e| fail(format!("cannot copy the lock: {e}")))?;
        let lock_json = |path: &Path| -> Result<Value, Refusal> {
            let b = std::fs::read(path).map_err(|e| fail(format!("cannot read the lock: {e}")))?;
            serde_json::from_slice(&b).map_err(|e| fail(format!("cannot read the lock: {e}")))
        };
        let l0 = lock_json(&lock_path)?;
        let base_path = p.base.to_string_lossy().into_owned();
        let (m, l) = (
            module.to_string_lossy().into_owned(),
            lock_path.to_string_lossy().into_owned(),
        );
        let mut picked: BTreeMap<String, String> = BTreeMap::new();
        for c in &cands {
            let tree = read_tree(&module);
            let lock_was = std::fs::read(&lock_path).map_err(|e| fail(e.to_string()))?;
            // A block an earlier adoption of this request pinned as an extern
            // is adopted under that name (lox moves the lines naming it into
            // its declaration); vislox's slug replaces it after.
            let now = lock_json(&lock_path)?;
            let as_slug = now["externals"]
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
            let refused = match &res {
                Err(e) => Some(e.clone()),
                Ok((false, o, e)) => Some(lox_says(o, e)),
                Ok((true, _, e)) => {
                    let warnings: Vec<&str> = e
                        .lines()
                        .filter(|l| l.trim_start().starts_with("warning"))
                        .collect();
                    // Anything but the module's `.loxl` files is not vislox's to write.
                    let other = read_tree(&module)
                        .into_iter()
                        .filter(|(rel, b)| !rel.ends_with(".loxl") && tree.get(rel) != Some(b))
                        .map(|(rel, _)| rel)
                        .next();
                    if !warnings.is_empty() {
                        Some(format!(
                            "lox warns, so it stays Loxone Config's:\n{}",
                            warnings.join("\n")
                        ))
                    } else {
                        other.map(|rel| {
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
                }
                Some(why) => {
                    // lox writes nothing when it refuses; after a warning
                    // or a file vislox does not write, the copy goes back.
                    write_tree(&module, &tree).map_err(fail)?;
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
        let l1 = lock_json(&lock_path)?;
        let own = |s: &str| files.block(s).is_some();
        let found = adopt::extract(
            &loxl_texts(&original),
            &loxl_texts(&read_tree(&module)),
            &l0,
            &l1,
            &picked,
            &own,
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
