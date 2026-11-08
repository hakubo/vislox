//! New logic pages (#88): `page "Küche Logik" (room: "Küche", category:
//! "Beleuchtung")` heads a file of vislox's, and `lox logic compile` creates
//! the page when the base has none with that title, as Loxone Config's "Neue
//! Seite" does (lox keeps its UUID in the lockfile's `pages`, so recompiles
//! create the same page until it is deployed). Blocks added there (#59) go
//! into the same file.
//!
//! - **`new_page`** writes the file: the title must be new (a page with that
//!   title would be used as is, so it is refused), `room` must name exactly
//!   one room of the base other than the unassigned one (Loxone Config needs
//!   a real room), and `category`, when given, exactly one category.
//! - **`remove_page`** takes the file out while the page is not deployed and
//!   holds nothing of vislox's. A deployed page is Loxone Config's: lox never
//!   deletes a page when its statement goes.
//! - **[`hand_off`]**: once the base or the deployed revision has the page,
//!   its line loses `room:` and `category:` (lox: *drop the arguments to hand
//!   the page to Loxone Config*), takes the `# page uuid:` line and follows
//!   the page's title, so that a rename in Loxone Config is followed, not
//!   refused. `actions.rs` runs it on disk before every compile vislox starts
//!   (an edit, Compile, Sync, Apply) and before Apply's and Sync's commits.

use crate::blocks::PageFile;
use crate::edit::{Applied, CLAIMS, Ctx, Files, Lock, Op, find_ci, pages_titled};
use loxone_config::LoxoneDoc;
use loxone_config::xml::Element;
use serde_json::{Value as Json, json};

/// The UUID of the page vislox's file `pf` creates (`room:` on its page
/// line): the lockfile's `pages` entry for its title, the one lox keeps
/// while it creates the page. A `# page uuid:` line is not trusted here: it
/// is written only once the page is handed off ([`hand_off`]), so on a file
/// that still creates its page it is stale. `None`: the file creates no
/// page, or lox has not created it yet.
pub fn created_uuid(pf: &PageFile, lock: &Lock) -> Option<String> {
    if pf.periphery || pf.room.is_none() {
        return None;
    }
    lock.pages.get(&pf.page).cloned()
}

/// The file of vislox's that creates the page `page` (case aside), and the
/// file itself.
pub fn created_file<'a>(
    files: &'a Files,
    lock: &Lock,
    page: &str,
) -> Option<(&'a String, &'a PageFile)> {
    files
        .pages
        .iter()
        .find(|(_, p)| created_uuid(p, lock).is_some_and(|u| u.eq_ignore_ascii_case(page)))
}

/// The page element with UUID `uuid` (case aside) in `doc`.
fn page_in<'a>(doc: &'a LoxoneDoc, uuid: &str) -> Option<&'a Element> {
    find_ci(&doc.xml.root, uuid).filter(|e| e.attr("Type") == Some("Page"))
}

/// Hand each page vislox's files create that one of `docs` (the base, the
/// deployed revision) has to Loxone Config: its line loses `room:` and
/// `category:`, takes the page's UUID (`# page uuid:`) and the title the
/// first of `docs` that has it gives it. The page is found by the UUID the
/// lockfile keeps for its title, else by its file's `# page uuid:` line.
/// Returns the names of the files it changed.
pub fn hand_off(files: &mut Files, docs: &[&LoxoneDoc], lock: &Lock) -> Vec<String> {
    let mut changed = Vec::new();
    for (name, p) in files.pages.iter_mut() {
        if p.periphery || p.room.is_none() {
            continue;
        }
        let ids = [lock.pages.get(&p.page).cloned(), p.uuid.clone()];
        let hit = ids.iter().flatten().find_map(|u| {
            docs.iter().find_map(|d| {
                let e = page_in(d, u)?;
                Some((
                    e.attr("U").unwrap_or(u).to_string(),
                    e.attr_decoded("Title").map(|t| t.into_owned()),
                ))
            })
        });
        let Some((uuid, title)) = hit else { continue };
        if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
            p.page = t;
        }
        p.uuid = Some(uuid);
        p.room = None;
        p.category = None;
        changed.push(name.clone());
    }
    changed
}

/// Follow each page Loxone Config renamed in `base` in vislox's page file
/// tied to it by UUID (`# page uuid:`): `page "…"` names the page by its
/// title, so a stale title fails the compile. Files that create their page
/// are [`hand_off`]'s. Returns the names of the files it changed.
pub fn follow_renames(files: &mut Files, base: &LoxoneDoc) -> Vec<String> {
    let mut changed = Vec::new();
    for (name, p) in files.pages.iter_mut() {
        if p.periphery || p.room.is_some() {
            continue;
        }
        let Some(t) = p
            .uuid
            .as_deref()
            .and_then(|u| page_in(base, u))
            .and_then(|e| e.attr_decoded("Title"))
            .filter(|t| !t.trim().is_empty() && *t != p.page)
        else {
            continue;
        };
        p.page = t.into_owned();
        changed.push(name.clone());
    }
    changed
}

/// The page with UUID `page` that a new statement of vislox's goes on (a
/// block, a signal, an instance: `what`), as `page "…"` names it: its title,
/// and vislox's file of it when there is one. A page of the base must be the
/// first with its title (lox builds on the first); a page created here (#88)
/// that the base does not have yet is its file's.
pub fn target(
    files: &Files,
    ctx: &Ctx,
    page: &str,
    what: &str,
) -> Result<(String, Option<String>), String> {
    let root = &ctx.base.xml.root;
    let Some(pg) = find_ci(root, page) else {
        return match created_file(files, ctx.lock, page) {
            Some((name, pf)) => Ok((pf.page.clone(), Some(name.clone()))),
            None => Err(format!("no page {page} in the project's base config")),
        };
    };
    if pg.attr("Type") != Some("Page") {
        return Err(format!("no page {page} in the project's base config"));
    }
    let title = pg.attr_decoded("Title").unwrap_or_default().into_owned();
    if title.trim().is_empty() {
        return Err("the page has no title; name it in Loxone Config first".into());
    }
    let mut same = Vec::new();
    pages_titled(root, &title, &mut same);
    if !same
        .first()
        .and_then(|e| e.attr("U"))
        .is_some_and(|u| u.eq_ignore_ascii_case(page))
    {
        return Err(format!(
            "{} pages are titled \"{title}\", and lox puts new blocks on the first; rename one in \
             Loxone Config to add {what} here",
            same.len()
        ));
    }
    let file = files
        .pages
        .iter()
        .find(|(_, p)| {
            !p.periphery
                && p.uuid
                    .as_deref()
                    .is_some_and(|u| u.eq_ignore_ascii_case(page))
        })
        .map(|(n, _)| n.clone());
    Ok((title, file))
}

/// Every titled object of type `ty` of `doc`, with whether it is the
/// unassigned one Loxone Config marks `First="true"`: one walk.
fn titled_of(doc: &LoxoneDoc, ty: &str) -> Vec<(String, bool)> {
    fn walk(el: &Element, ty: &str, out: &mut Vec<(String, bool)>) {
        if el.attr("Type") == Some(ty)
            && let Some(t) = el.attr_decoded("Title")
            && !t.trim().is_empty()
        {
            out.push((t.into_owned(), el.attr("First") == Some("true")));
        }
        for c in el.child_elements() {
            walk(c, ty, out);
        }
    }
    let mut all = Vec::new();
    walk(&doc.xml.root, ty, &mut all);
    all
}

/// The objects of type `ty` (`Place`, `Category`) of `doc` that a page line
/// may name: titled, the title no other's of the type, and (`real`) not the
/// unassigned one. Sorted by title.
pub fn choices(doc: &LoxoneDoc, ty: &str, real: bool) -> Vec<String> {
    let all = titled_of(doc, ty);
    let mut out: Vec<String> = all
        .iter()
        .filter(|(t, first)| !(real && *first) && all.iter().filter(|(o, _)| o == t).count() == 1)
        .map(|(t, _)| t.clone())
        .collect();
    out.sort_by_key(|t| t.to_lowercase());
    out
}

/// Why `title` cannot name the room (`Place`) or category of a new page, as
/// lox resolves it against the base: exactly one of its type, and a room not
/// the unassigned one.
fn taxonomy(base: &LoxoneDoc, ty: &str, what: &str, title: &str) -> Result<(), String> {
    if title.trim().is_empty() {
        return Err(format!("give the page a {what}"));
    }
    let hits: Vec<bool> = titled_of(base, ty)
        .into_iter()
        .filter(|(t, _)| t == title)
        .map(|(_, first)| first)
        .collect();
    match hits.as_slice() {
        [] => Err(format!(
            "no {what} is titled \"{title}\" in the project's base config; make it in Loxone \
             Config first"
        )),
        [true] if ty == "Place" => Err(format!(
            "Loxone Config needs a real room for a page: name a room other than \"{title}\""
        )),
        [_] => Ok(()),
        many => Err(format!(
            "{} {what}s are titled \"{title}\"; rename one in Loxone Config",
            many.len()
        )),
    }
}

/// A page's title as a page line takes it: one line, not empty, no spaces at
/// its ends (the title is what lox looks the page up by), and kept as it is
/// by a Loxone Config save (lox's `title_save_rewrite`, as for a block's name).
fn page_title(t: &str) -> Result<&str, String> {
    if t.chars().any(char::is_control) {
        return Err("the page's title must be one line of text".into());
    }
    if t.trim().is_empty() {
        return Err("give the page a title".into());
    }
    if t.trim() != t {
        return Err("the page's title has spaces at its ends".into());
    }
    if t.chars().count() > 120 {
        return Err("the page's title is longer than 120 characters".into());
    }
    if let Some(w) = loxone_config::connectors::title_save_rewrite("Page", t) {
        return Err(format!(
            "the page's title: a Loxone Config save would write it as \"{w}\"; write that instead"
        ));
    }
    Ok(t)
}

/// `new_page`: a file of vislox's headed `page "title" (room: …, category:
/// …)`. Everything is checked before `files` changes.
pub fn new_page(
    files: &mut Files,
    ctx: &Ctx,
    title: &str,
    room: &str,
    category: Option<&str>,
) -> Result<Applied, String> {
    let title = page_title(title)?;
    // A page with the title is used as is: it would not be a new page.
    for (doc, which) in [(ctx.base, "base"), (ctx.doc, "pending")] {
        let mut same = Vec::new();
        pages_titled(&doc.xml.root, title, &mut same);
        if !same.is_empty() {
            return Err(format!(
                "the {which} config has a page titled \"{title}\" already; add blocks to it, or \
                 give the new page another title"
            ));
        }
    }
    // A file of vislox's for the title: only one `remove_page` left with the
    // `removed` lines lox still needs is taken again.
    let left = files
        .pages
        .iter()
        .find(|(_, p)| !p.periphery && p.page == title)
        .map(|(n, p)| (n.clone(), p.room.is_none() && !p.holds(&|_| true)));
    if let Some((name, false)) = &left {
        return Err(format!(
            "vislox's file {name} is for a page titled \"{title}\" already"
        ));
    }
    taxonomy(ctx.base, "Place", "room", room)?;
    let category = category.filter(|c| !c.is_empty());
    if let Some(c) = category {
        taxonomy(ctx.base, "Category", "category", c)?;
    }
    // Everything is checked: from here on `files` changes.
    let name = match left {
        Some((n, _)) => n,
        None => crate::blocks::file_name(title, &|n| n == CLAIMS || files.pages.contains_key(n)),
    };
    let f = files
        .pages
        .entry(name)
        .or_insert_with(|| PageFile::new(title));
    f.uuid = None;
    f.room = Some(room.to_string());
    f.category = category.map(String::from);
    Ok(Applied {
        previous: Json::Null,
        added: None,
        warnings: Vec::new(),
    })
}

/// What `remove_page` of `page` takes out (the file's name) and the
/// `new_page` that undoes it, or why it is refused. It changes nothing.
/// `removed` lines of blocks vislox added on the page (the lock pins them to
/// it, or forgot them) do not keep it: an undo of a new page and a block on
/// it is one request.
pub fn removal(
    files: &Files,
    base: &LoxoneDoc,
    lock: &Lock,
    page: &str,
) -> Result<(String, Op), String> {
    let (name, pf) = created_file(files, lock, page).ok_or_else(|| {
        format!("vislox created no page {page}; a page Loxone Config made is deleted there")
    })?;
    let title = &pf.page;
    if page_in(base, page).is_some() {
        return Err(format!(
            "\"{title}\" is deployed, and lox never deletes a page; delete it in Loxone Config"
        ));
    }
    let here = |slug: &str| match lock.placed.get(slug) {
        None => !lock.slugs.contains_key(slug),
        Some(at) => at.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(page)),
    };
    if pf.holds(&here) {
        return Err(format!(
            "\"{title}\" still has what vislox added there ({name}); take that out first"
        ));
    }
    let undo = Op::NewPage {
        title: title.clone(),
        room: pf.room.clone().unwrap_or_default(),
        category: pf.category.clone(),
        restores: Some(page.to_string()),
    };
    Ok((name.clone(), undo))
}

/// `remove_page`: the page line loses `room:`, so the compile creates the
/// page no more, and the file goes, unless lox still needs its `removed`
/// lines (then it stays without them until [`Files::tidy`] drops them). Its
/// `previous` is the `new_page` that creates the page again (lox mints a new
/// UUID: `restores`).
pub fn remove_page(files: &mut Files, ctx: &Ctx, page: &str) -> Result<Applied, String> {
    let (name, undo) = removal(files, ctx.base, ctx.lock, page)?;
    let f = files.pages.get_mut(&name).expect("checked");
    f.room = None;
    f.category = None;
    f.uuid = None;
    if f.is_empty() {
        files.pages.remove(&name);
    }
    Ok(Applied {
        previous: serde_json::to_value(&undo).unwrap_or_default(),
        added: None,
        warnings: Vec::new(),
    })
}

/// `GET /api/edits`' `pages`: the rooms and categories a new page may name
/// (the base's, as lox resolves them; not the unassigned ones), and each page
/// vislox's files create: its UUID (null until compiled), title, room,
/// category and file, whether the base has it (`deployed`: the config lox
/// compiles against, the one `remove_page` asks too), and whether
/// `remove_page` takes it out now (null) or why not.
pub fn edits_section(files: &Files, base: &LoxoneDoc, lock: &Lock, dir: &str) -> Json {
    let mine: Vec<(&String, &PageFile)> = files
        .pages
        .iter()
        .filter(|(_, p)| !p.periphery && p.room.is_some())
        .collect();
    // As an edit sees the files: the `removed` and `moved` lines lox needs no
    // more go first (`Files::tidy`), so an emptied page can go at once. Only
    // when there is a page to ask about.
    let tidied = (!mine.is_empty()).then(|| {
        let mut t = files.clone();
        t.tidy(lock);
        t
    });
    let created: Vec<Json> = mine
        .into_iter()
        .map(|(name, p)| {
            let uuid = created_uuid(p, lock);
            let remove = match (&uuid, &tidied) {
                (Some(u), Some(t)) => removal(t, base, lock, u).err(),
                _ => Some("lox has not created it yet: compile the module".to_string()),
            };
            json!({
                "uuid": uuid,
                "title": p.page,
                "room": p.room,
                "category": p.category,
                "file": format!("{dir}/{name}"),
                "deployed": uuid.as_deref().is_some_and(|u| page_in(base, u).is_some()),
                "remove": remove,
            })
        })
        .collect();
    json!({
        "rooms": choices(base, "Place", true),
        // No category files it under the unassigned one: not offered by name.
        "categories": choices(base, "Category", true),
        "created": created,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::edit::{NO_TEMPLATES, apply_op};
    use std::collections::{BTreeMap, BTreeSet};

    pub(crate) const KUECHE: &str = "30000a01-0000-0000-ffff504f94000001";
    const OG: &str = "10000008-0000-0080-ffff504f94000001";
    const NEU: &str = "21941aa8-0000-0000-ffff504f94000001";

    /// The test house with rooms (the unassigned one, Küche, two named Flur)
    /// and categories (the unassigned one, Beleuchtung).
    pub(crate) fn house() -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/house.Loxone",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
        .replacen(
            "\t\t<C Type=\"Page\" V=\"178\" U=\"10000002",
            &format!(
                "\t\t<C Type=\"PlaceCaption\" V=\"178\" U=\"30000a00-0000-0000-ffff504f94000001\" Title=\"Räume\">\n\
                 \t\t\t<C Type=\"Place\" V=\"178\" U=\"30000a09-0000-0000-ffff504f94000001\" Title=\"Nicht zugeordnet\" First=\"true\"/>\n\
                 \t\t\t<C Type=\"Place\" V=\"178\" U=\"{KUECHE}\" Title=\"Küche\"/>\n\
                 \t\t\t<C Type=\"Place\" V=\"178\" U=\"30000a04-0000-0000-ffff504f94000001\" Title=\"Flur\"/>\n\
                 \t\t\t<C Type=\"Place\" V=\"178\" U=\"30000a05-0000-0000-ffff504f94000001\" Title=\"Flur\"/>\n\
                 \t\t</C>\n\
                 \t\t<C Type=\"CategoryCaption\" V=\"178\" U=\"30000b00-0000-0000-ffff504f94000001\" Title=\"Kategorien\">\n\
                 \t\t\t<C Type=\"Category\" V=\"178\" U=\"30000b09-0000-0000-ffff504f94000001\" Title=\"Nicht zugeordnet\" First=\"true\"/>\n\
                 \t\t\t<C Type=\"Category\" V=\"178\" U=\"30000b01-0000-0000-ffff504f94000001\" Title=\"Beleuchtung\"/>\n\
                 \t\t</C>\n\
                 \t\t<C Type=\"Page\" V=\"178\" U=\"10000002"
            ),
            1,
        )
    }

    /// `house()` with the page lox created from the line, as the pending
    /// config has it.
    fn with_page(xml: &str, title: &str) -> String {
        xml.replacen(
            "\t\t<C Type=\"Page\" V=\"178\" U=\"10000002",
            &format!(
                "\t\t<C Type=\"Page\" V=\"178\" U=\"{NEU}\" Title=\"{title}\" WF=\"16384\"/>\n\t\t<C Type=\"Page\" V=\"178\" U=\"10000002"
            ),
            1,
        )
    }

    fn ctx<'a>(doc: &'a LoxoneDoc, base: &'a LoxoneDoc, lock: &'a Lock) -> Ctx<'a> {
        static NAMES: std::sync::LazyLock<BTreeSet<String>> =
            std::sync::LazyLock::new(BTreeSet::new);
        static DRAWN: std::sync::LazyLock<BTreeSet<crate::edit::Wire>> =
            std::sync::LazyLock::new(BTreeSet::new);
        static AD: std::sync::LazyLock<BTreeMap<String, crate::adopt::Adoption>> =
            std::sync::LazyLock::new(BTreeMap::new);
        static PAD: std::sync::LazyLock<BTreeMap<String, Vec<String>>> =
            std::sync::LazyLock::new(BTreeMap::new);
        static CL: std::sync::LazyLock<crate::set_claim::Claimed> =
            std::sync::LazyLock::new(Default::default);
        Ctx {
            doc,
            base,
            lock,
            deployed: Some(base),
            names: &NAMES,
            lets: &NAMES,
            drawn: &DRAWN,
            adoptions: &AD,
            page_adoptions: &PAD,
            claimed: &CL,
            templates: &NO_TEMPLATES,
            module: &crate::module_edit::NO_MODULE,
        }
    }

    fn new(title: &str, room: &str, category: Option<&str>) -> Op {
        Op::NewPage {
            title: title.into(),
            room: room.into(),
            category: category.map(String::from),
            restores: None,
        }
    }

    #[test]
    fn a_new_page_is_a_file_headed_by_its_page_line() {
        let base = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let lock = Lock::default();
        let c = ctx(&base, &base, &lock);
        let mut files = Files::default();
        let a = apply_op(
            &mut files,
            &c,
            &new("Küche Logik", "Küche", Some("Beleuchtung")),
        )
        .unwrap();
        assert_eq!(a.previous, Json::Null);
        let text = &files.render()["kueche-logik.loxl"];
        assert!(
            text.ends_with("page \"Küche Logik\" (room: \"Küche\", category: \"Beleuchtung\")\n"),
            "{text}"
        );
        // Read back as vislox reads its files: the same file.
        let back = Files::from_texts(&files.render()).unwrap();
        assert_eq!(back.pages, files.pages);
        // Without a category, lox files it under the unassigned one.
        let mut files = Files::default();
        apply_op(&mut files, &c, &new("Licht", "Küche", None)).unwrap();
        assert!(files.render()["licht.loxl"].ends_with("page \"Licht\" (room: \"Küche\")\n"));
    }

    #[test]
    fn a_new_page_is_checked_before_anything_changes() {
        let base = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let pending = LoxoneDoc::parse(with_page(&house(), "Schon da").as_bytes()).unwrap();
        let lock = Lock::default();
        let c = ctx(&pending, &base, &lock);
        let mut files = Files::default();
        let mut err = |op: Op| apply_op(&mut files, &c, &op).unwrap_err();
        assert!(err(new("Obergeschoss", "Küche", None)).contains("base config has a page"));
        assert!(err(new("Schon da", "Küche", None)).contains("pending config has a page"));
        assert!(err(new("", "Küche", None)).contains("title"));
        assert!(err(new(" Neu", "Küche", None)).contains("spaces at its ends"));
        assert!(err(new("A\nB", "Küche", None)).contains("one line"));
        assert!(err(new("Neu", "", None)).contains("give the page a room"));
        assert!(err(new("Neu", "Keller", None)).contains("no room is titled \"Keller\""));
        assert!(err(new("Neu", "Flur", None)).contains("2 rooms are titled"));
        assert!(err(new("Neu", "Nicht zugeordnet", None)).contains("real room"));
        assert!(err(new("Neu", "Küche", Some("Heizung"))).contains("no category"));
        assert_eq!(files, Files::default());
        // A file of vislox's for that title already.
        apply_op(&mut files, &c, &new("Neu", "Küche", None)).unwrap();
        let again = apply_op(&mut files, &c, &new("Neu", "Küche", None)).unwrap_err();
        assert!(again.contains("neu.loxl is for a page"), "{again}");
    }

    #[test]
    fn the_choices_are_the_rooms_and_categories_a_page_line_may_name() {
        let base = LoxoneDoc::parse(house().as_bytes()).unwrap();
        assert_eq!(choices(&base, "Place", true), ["Küche"]);
        assert_eq!(
            choices(&base, "Category", false),
            ["Beleuchtung", "Nicht zugeordnet"]
        );
        assert_eq!(choices(&base, "Category", true), ["Beleuchtung"]);
    }

    /// The page lox created, as the lock names it: blocks go into its file,
    /// it is removed while it holds nothing, and its undo creates it again.
    #[test]
    fn a_created_page_takes_blocks_and_is_removed_while_empty() {
        let base = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let pending = LoxoneDoc::parse(with_page(&house(), "Küche Logik").as_bytes()).unwrap();
        let mut lock = Lock::default();
        lock.pages.insert("Küche Logik".into(), NEU.into());
        let c = ctx(&pending, &base, &lock);
        // What `new_page` wrote, compiled: the pending config has the page.
        let mut files = Files::default();
        files.pages.insert(
            "kueche-logik.loxl".into(),
            PageFile {
                room: Some("Küche".into()),
                category: Some("Beleuchtung".into()),
                ..PageFile::new("Küche Logik")
            },
        );
        // The page by its UUID: its file, case aside.
        let (title, file) = target(&files, &c, &NEU.to_uppercase(), "a block").unwrap();
        assert_eq!(
            (title.as_str(), file.as_deref()),
            ("Küche Logik", Some("kueche-logik.loxl"))
        );
        // A block added there goes into the same file.
        let add = Op::AddBlock {
            page: NEU.into(),
            ty: "And".into(),
            title: None,
            params: Default::default(),
            settings: Default::default(),
            wires: Vec::new(),
            restores: None,
        };
        apply_op(&mut files, &c, &add).unwrap();
        assert_eq!(files.pages.len(), 1);
        let text = &files.render()["kueche-logik.loxl"];
        assert!(
            text.contains("(room: \"Küche\", category: \"Beleuchtung\")\n\nvx_and_1 = And()\n"),
            "{text}"
        );
        let remove = Op::RemovePage { page: NEU.into() };
        let err = apply_op(&mut files.clone(), &c, &remove).unwrap_err();
        assert!(err.contains("still has what vislox added"), "{err}");
        // Taken out again: its `removed` line, which lox needs no more once the
        // lock forgets the block, does not keep the page.
        let pf = files.pages.get_mut("kueche-logik.loxl").unwrap();
        pf.blocks.clear();
        pf.removed.insert("vx_and_1".into());
        let s = edits_section(&files, &base, &lock, "pages/vislox");
        assert_eq!(s["created"][0]["remove"], Json::Null);
        // One the lock still pins to the page does not either (an undo of the
        // page and a block on it, in one request); one on another page does.
        let mut pinned = lock.clone();
        pinned.slugs.insert(
            "vx_and_1".into(),
            "bbbbbb01-0000-0000-ffff504f94000001".into(),
        );
        pinned
            .placed
            .insert("vx_and_1".into(), Some(NEU.to_uppercase()));
        assert!(removal(&files, &base, &pinned, NEU).is_ok());
        pinned.placed.insert("vx_and_1".into(), Some(OG.into()));
        assert!(
            removal(&files, &base, &pinned, NEU)
                .unwrap_err()
                .contains("still has")
        );
        files.tidy(&lock);
        let a = apply_op(&mut files, &c, &remove).unwrap();
        assert!(files.pages.is_empty());
        let undo: Op = serde_json::from_value(a.previous).unwrap();
        assert_eq!(
            undo,
            Op::NewPage {
                title: "Küche Logik".into(),
                room: "Küche".into(),
                category: Some("Beleuchtung".into()),
                restores: Some(NEU.into()),
            }
        );
        // A page Loxone Config made, or one never created, is not removed here.
        assert!(
            apply_op(&mut files, &c, &Op::RemovePage { page: OG.into() })
                .unwrap_err()
                .contains("vislox created no page")
        );
    }

    /// Once the base has the page, it is Loxone Config's: not removed here,
    /// and its line loses `room:` so that a rename there is followed.
    #[test]
    fn a_deployed_page_is_handed_to_loxone_config() {
        let deployed = LoxoneDoc::parse(with_page(&house(), "Küche Logik").as_bytes()).unwrap();
        let mut lock = Lock::default();
        lock.pages.insert("Küche Logik".into(), NEU.into());
        let mut files = Files::default();
        files.pages.insert(
            "kueche-logik.loxl".into(),
            PageFile {
                room: Some("Küche".into()),
                ..PageFile::new("Küche Logik")
            },
        );
        let err = removal(&files, &deployed, &lock, NEU).unwrap_err();
        assert!(err.contains("is deployed"), "{err}");
        let s = edits_section(&files, &deployed, &lock, "pages/vislox");
        assert_eq!(s["created"][0]["uuid"], NEU);
        assert_eq!(s["created"][0]["deployed"], true);
        assert!(
            s["created"][0]["remove"]
                .as_str()
                .unwrap()
                .contains("deployed")
        );
        // Renamed in Loxone Config: handed off, the file follows it without
        // `room:`, and takes the UUID; later renames are followed by it.
        let renamed = LoxoneDoc::parse(with_page(&house(), "Kochen").as_bytes()).unwrap();
        assert_eq!(
            hand_off(&mut files, &[&renamed], &lock),
            ["kueche-logik.loxl"]
        );
        let p = &files.pages["kueche-logik.loxl"];
        assert_eq!((p.page.as_str(), p.uuid.as_deref()), ("Kochen", Some(NEU)));
        assert_eq!(p.room, None);
        assert!(hand_off(&mut files, &[&renamed], &lock).is_empty());
        let again = LoxoneDoc::parse(with_page(&house(), "Kochen 2").as_bytes()).unwrap();
        assert_eq!(follow_renames(&mut files, &again), ["kueche-logik.loxl"]);
        assert_eq!(files.pages["kueche-logik.loxl"].page, "Kochen 2");
        // Nothing left in it: the file goes, the page stays.
        assert!(files.render().is_empty());
        // Not in the base, but in the deployed revision: handed off too.
        let mut files2 = Files::default();
        files2.pages.insert(
            "kueche-logik.loxl".into(),
            PageFile {
                room: Some("Küche".into()),
                ..PageFile::new("Küche Logik")
            },
        );
        let base = LoxoneDoc::parse(house().as_bytes()).unwrap();
        assert!(hand_off(&mut files2, &[&base], &lock).is_empty());
        assert_eq!(hand_off(&mut files2, &[&base, &deployed], &lock).len(), 1);
    }

    /// A `# page uuid:` line on a file that still creates its page is stale
    /// (lox may mint another UUID after a Discard): the lock's UUID counts,
    /// and the line is not written again.
    #[test]
    fn a_stale_page_uuid_line_does_not_hide_the_created_page() {
        let mut lock = Lock::default();
        lock.pages.insert("Neu".into(), NEU.into());
        let pf = PageFile {
            uuid: Some("deadbeef-0000-0000-ffff504f94000001".into()),
            room: Some("Küche".into()),
            ..PageFile::new("Neu")
        };
        assert_eq!(created_uuid(&pf, &lock).as_deref(), Some(NEU));
        assert!(!pf.render().contains("# page uuid:"), "{}", pf.render());
        assert_eq!(created_uuid(&pf, &Lock::default()), None);
    }

    /// Not compiled yet, the page has no UUID: listed, not removable.
    #[test]
    fn a_page_lox_has_not_created_is_listed_without_a_uuid() {
        let base = LoxoneDoc::parse(house().as_bytes()).unwrap();
        let mut files = Files::default();
        files.pages.insert(
            "neu.loxl".into(),
            PageFile {
                room: Some("Küche".into()),
                ..PageFile::new("Neu")
            },
        );
        let s = edits_section(&files, &base, &Lock::default(), "pages/vislox");
        assert_eq!(s["rooms"], json!(["Küche"]));
        assert_eq!(s["created"][0]["uuid"], Json::Null);
        assert_eq!(s["created"][0]["file"], "pages/vislox/neu.loxl");
        assert!(s["created"][0]["remove"].is_string());
    }
}
