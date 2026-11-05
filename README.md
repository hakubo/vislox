<img src="assets/icon.svg" width="64" height="64" alt="">

# vislox

A read-only, Loxone Config-style view of the Miniserver configs that
[lox](https://github.com/discostu105/lox) and its `loxone-config` crate manage. You see the logic pages
the way Loxone Config draws them. The view reloads when the config file changes, and every change is
marked so you can review it before you deploy.

- **Pages as tabs**: one sheet tab per logic page along the bottom, as in Loxone Config. Each tab shows how
  many changes it holds.
- **Blocks, refs and wires** in the Loxone Config 16 look: rounded blocks with a coloured header, grey port
  stubs, constants beside unwired inputs, and pill-shaped references with their address and an analog (`#`) or
  digital (◉) mark. Block footprints, port rows and wire routing come from
  `loxone_config::layout`, the same geometry `lox` uses to place blocks. Input and output refs show as
  reference bars with their short address (`VI1`, `AQ1`, …).
- **Links between pages**: when a wire's far end is on another page, the port gets a reference bar with
  the far block, its port and its page. Click it to go to that page. Double-click a ref tag to go to the
  block it mirrors.
- **Sidebar with the block's configuration**: the periphery and page tree (filter with Ctrl+F), and the
  selected block's properties: name, type, page, room, category, connections (each one a link),
  parameters and raw attributes.
- **History** (right): every revision, newest first:
  - *Pending*: the compiled `out.Loxone` of a `lox.toml` project, compared with the deployed `base`.
    This is what `lox config push` would apply.
  - *Changed on disk*: snapshots taken while vislox runs, whenever the file changes.
  - *Git commits*: the file's history, e.g. from `lox config pull`.
- **Change review**: added (green), changed (amber), removed (red, dashed ghost) and moved (blue, ghost at
  the old place) blocks. Wires and parameters are marked too: changed values show old → new in the
  properties. The Changes list can be filtered by kind, and clicking an entry jumps to it.
- **Readable at every zoom**: zoomed out, each block shows only its name, at a readable size. From about 80 %
  the full detail appears. Gates show their operation (`AND`, `Less`, `Divide`) instead of Loxone's automatic names
  (`O208698`); a named gate keeps its operation in grey, as Loxone Config 16 shows it. Selecting a block highlights its wires and neighbours and dims the rest.
- **Navigation**: Ctrl+K finds any page or block. There is a minimap, a "Changed pages only" filter, and links
  (`#page=…&sel=…&rev=…`) that reopen the same page, block and revision. The browser's Back button works.
- **Keyboard**: the tree, tabs and canvas all work from the keyboard; press `?` for the shortcuts.

Read-only: vislox never writes to the config.

## Run

```bash
cargo install --path .          # or: cargo run --release -- <path>

vislox ~/loxone-config          # a `lox config init` repository (<serial>/config.Loxone)
vislox ./my-project             # a lox.toml project: base vs compiled out.Loxone
vislox config.Loxone            # one .Loxone file, or a backup .zip
vislox ~/loxone-config --serial 504F94AABBCC   # several Miniservers in one repository
```

Then open <http://127.0.0.1:7878> (`--listen` changes the address, and `--open` launches the browser).

| Input | Click | Shift+click |
|---|---|---|
| History entry | view it, compared with the entry before it | compare the viewed revision with this one |

The *Viewing* and *compared with* menus in the toolbar pick any pair of revisions. Pick *nothing* to see
a revision without change marks. *Follow latest* jumps to each new revision as it appears.

Keys:
- `Ctrl+K`: go to a page or block. `Ctrl+F`: filter the tree. `?`: list all shortcuts.
- `[` / `]`: previous / next change. PageUp / PageDown: switch pages.
- Arrow keys: pan. `+` / `-` / `0` / `1`: zoom in, zoom out, fit, 100 %.
- Esc: clear the selection.

With the mouse, drag to pan and scroll to zoom.

## How it works

The binary is an `axum` server with the UI embedded (plain JS and SVG, no build step).

- `src/source.rs` resolves the path into the watched files.
- `src/history.rs` keeps revisions keyed by git blob id. A live snapshot and the commit with the same
  bytes therefore match, and git content is read with `git cat-file` only when needed.
- `src/model.rs` turns a revision, plus an optional base, into pages, objects, wires, cross-page links,
  properties and a change list. It uses `loxone_config::diff` and adds its own checks for attributes,
  positions and wires.
- File changes are noticed with `notify`. Git is polled every 3 s. Browsers get a server-sent event and
  reload.

`docs/reference/` holds the Loxone Config screenshots the look is based on (see `SOURCES.md`). They are
copyright Loxone and are for internal reference only.

## Development

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test                                   # model, history, source; fixture: tests/fixtures/house.Loxone
cargo build --release
cd tests/ui && npm ci && npx playwright install chromium && cd ../..
VISLOX_BIN=target/release/vislox node tests/ui/smoke.mjs   # drives the UI in headless Chromium
```

CI (`.github/workflows/ci.yml`) runs all of these on every push to `main` and on every pull request.
