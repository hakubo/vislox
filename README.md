<img src="assets/icon.svg" width="64" height="64" alt="">

# vislox

A read-only, Loxone Config-style view of the Miniserver configs that
[lox](https://github.com/discostu105/lox) and its `loxone-config` crate manage. You see the logic pages
the way Loxone Config draws them. The view reloads when the config file changes, and every change is
marked so you can review it before you deploy.

- **Pages as tabs**: one sheet tab per logic page along the bottom, as in Loxone Config. Each tab shows how
  many changes it holds.
- **Blocks, refs and wires**: block footprints, port rows and wire routing come from
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
  - **Pull from Miniserver** (in a `lox config init` repository): runs `lox config pull`, which downloads the
    Miniserver's current config and commits it when it changed. The new backup appears at once.
- **Change review**: added (green), changed (amber), removed (red, dashed ghost) and moved (blue, ghost at
  the old place) blocks. Wires and parameters are marked too: changed values show old → new in the
  properties. The Changes list can be filtered by kind, and clicking an entry jumps to it.
- **Readable at every zoom**: zoomed out, each block shows only its name, at a readable size. From about 80 %
  the full detail appears. Gates show their operation (`AND`, `≥`, `f(x)`) instead of Loxone's automatic names
  (`O208698`). Selecting a block highlights its wires and neighbours and dims the rest.
- **Navigation**: Ctrl+K finds any page or block. There is a minimap, a "Changed pages only" filter, and links
  (`#page=…&sel=…&rev=…`) that reopen the same page, block and revision. The browser's Back button works.
- **Keyboard**: the tree, tabs and canvas all work from the keyboard; press `?` for the shortcuts.

- **Live values** (toggle in the toolbar, or `L`): the Miniserver's current values on the wires, as in
  Loxone Config's Liveview. Digital wires that are on turn green, and analog and text values are written at
  the wire's source and flash when they change. The selected block's states and port values are listed in
  the sidebar. The Miniserver only streams what the app sees (visualized blocks and the periphery). vislox
  fills in more:
  - input and output references and memory flags carry the value of what feeds them, as do modes and
    unscaled system variables set through an output reference, and constants show their value from the
    config;
  - simple stateless gates (AND, OR, XOR, NOT, comparisons, Add, Subtract, Multiply, Divide) are computed
    from their inputs, and a lighting controller's Mood output from its active mood. Computed values have a
    dashed outline and say so in the tooltip.

  Blocks with timing or memory (delays, edge detection, Formula, …) and inputs the app does not show stay
  without a value. Those wires are faded ("Dim wires without a value" in the legend turns this off). Values
  show on the current, deployed revision only.

- **Change with AI, then Apply** (for a `lox.toml` project, when started with an admin token): describe a change
  in words. vislox sends it to a webhook, where an AI agent edits the project's Lox Logic module with `lox logic`
  and compiles it. The result shows as the pending revision, so you review it like any compile. **Apply** deploys
  it with `lox config push --force` (download, drift check, upload, program restart, verify), then commits the
  project and pushes it to its git remote. The page you are on goes with the request, and so do blocks you pick
  (Alt+click on the canvas, or **＋ AI context** in Properties), so the agent knows where to look. **Discard** puts the module back to the last commit. **Sync from
  Miniserver** makes what the Miniserver runs now the project's base (`lox config merge`: changes saved from
  Loxone Config, rules edited in the app), then commits and pushes; it waits until nothing is pending.

vislox itself never writes to the config: lox and git do, and only on Pull, Apply and Discard. Live values only
listen. A pull reads from the Miniserver; only Apply writes to it.

## Run

```bash
cargo install --path .          # or: cargo run --release -- <path>

vislox ~/loxone-config          # a `lox config init` repository (<serial>/config.Loxone)
vislox ./my-project             # a lox.toml project: base vs compiled out.Loxone
vislox config.Loxone            # one .Loxone file, or a backup .zip
vislox ~/loxone-config --serial 504F94AABBCC   # several Miniservers in one repository
```

Then open <http://127.0.0.1:7878> (`--listen` changes the address, and `--open` launches the browser).

Live values use the Miniserver connection of `lox` (`lox setup set …`), found the way lox finds it:
`LOX_CONFIG`, a project's `.lox/config.yaml`, then `~/.lox/config.yaml`. With several contexts, vislox picks
the one for the Miniserver it shows (a repository's `<serial>` or `192_168_1_77` folder), else the active one.

```bash
vislox ~/loxone-config --ctx home      # pick a lox context
vislox ~/loxone-config --no-live       # never connect to the Miniserver
lox stream -o json --initial --reconnect | vislox ~/loxone-config --live-from -   # values from lox
vislox config.Loxone --live-from recorded.ndjson   # replay a recording (followed as it grows)
```

### Change with AI and Apply

For a `lox.toml` project:

```bash
vislox ~/loxone-config/project --admin-token-file ~/.config/vislox/admin-token \
       --ai-webhook-file ~/.config/vislox/ai-webhook
```

- `--admin-token-file`: a file holding a passphrase. The AI box, Apply and Discard ask for it once per browser
  (`X-Vislox-Token`). Without it they are off; viewing needs no token.
- `--ai-webhook-file`: a file holding the URL that requests are POSTed to, as JSON: `id`, `key`, `prompt`,
  `context` (plain lines naming the page on screen and the picked blocks, with their UUIDs), `project` and
  `status_url`. The agent behind it reports back by POSTing `{"id", "key", "state": "working" |
  "done" | "failed", "summary"}` to `status_url`; the key authorizes only that request. A request that never
  reports back is given up after 30 minutes.
- The agent should only edit and compile (`lox logic …`). Deploying is Apply's job, and Apply needs the token.
- Apply commits with the change summary as the subject (`lox logic diff --summary`) and the AI requests in the
  body, then runs `git push`, which needs credentials for the project's remote.

The Miniserver connection is opened when a browser turns live values on, and closed shortly after the last
one turns them off.

**Pull from Miniserver** runs `lox config pull` (the `lox` on `PATH`, or `--lox <path>`), with the same lox
configuration and context as live values. It is offered only when lox would commit to the repository vislox
shows, into the folder of the Miniserver vislox shows (`config_repo` and the serial or address in lox's
configuration). Otherwise the button is greyed out and its tooltip says why. `--no-pull` hides it.

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
- `src/actions.rs` sends AI requests to the webhook, takes the agent's reports, and runs Apply (`lox config push
  --force`, `git commit`, `git push`), Sync (`lox config merge`, `git commit`, `git push`) and Discard
  (`git checkout` of the module, lock and out).
- `src/pull.rs` runs `lox config pull` for the Pull button (`POST /api/pull`, which needs an `X-Vislox` header so
  that no other web page can trigger it), then refreshes the history.
- `src/live.rs` streams the Miniserver's states with lox's `loxone-client` crate. A wire's source
  connector has the UUID of the state it carries, so the browser matches values to wires by UUID. Values
  reach the browser on `/api/live` (server-sent events).
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
