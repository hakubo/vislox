# Editing in vislox

vislox becomes an editor in steps: edit values (#58), add blocks (#59), move blocks, inputs and outputs (#60),
make and change connections (#61), and undo and redo (#62). This page is the design they share. #58, #59 and #60
are built; the rest says where each step plugs in.

## The rules

- **Only a `lox.toml` project is edited.** It is the only source with a pending revision, Apply and Discard.
  A `.Loxone` file, a backup or a `lox config init` repository stays read-only.
- **Edits go into the module, never into a `.Loxone`.** `lox config push` compiles the module again before it
  uploads, so a change made only in `out.Loxone` would be lost on Apply, and `base.Loxone` is what the
  Miniserver runs.
- **vislox writes one place: its own files under the module directory**, `<module>/vislox/`: `edits.loxl` for
  the values it claims, and one file per page for the blocks it adds (`<module>/vislox/obergeschoss.loxl`).
  The rest of the module is never touched. A module that is one file cannot be edited from vislox.
  The one exception is a move (#60): lox keeps a managed block's position in the lockfile, not in the source,
  so vislox writes it there, the way lox's own `--relayout` does (see *Moving blocks*).
- **Every edit is compiled with `lox logic compile`** and shows as the *Pending* revision. Change review, Apply,
  Discard and Sync work on it as on any other compile.
- **Editing never deploys.** It runs `lox logic compile` and nothing else, and sends nothing to the Miniserver.
  Only Apply deploys, with its own preflight and confirmation.
- **Editing needs the admin token**, like Apply (`--admin-token-file`, `X-Vislox-Token`).

## The edit model

An edit is a list of operations against a base revision:

```json
POST /api/edit
{ "base": "<blob id of the config shown>",
  "ops": [ { "op": "title",   "uuid": "…", "value": "Teiler" },
           { "op": "param",   "uuid": "…", "port": "Input2", "value": "100" },
           { "op": "setting", "uuid": "…", "name": "MinTime", "value": "500" },
           { "op": "add_block", "page": "<page uuid>", "type": "Monoflop", "title": "Nachlauf",
             "params": { "Time": "30" }, "settings": {} },
           { "op": "remove_block", "uuid": "…" },
           { "op": "move", "uuid": "…", "x": 1920, "y": 960 },
           { "op": "relayout", "page": "<page uuid>" } ] }
```

- `base` is the git blob id of what the browser showed: the newest compile, or the deployed config when nothing
  is pending. When the file on disk is another one (someone compiled, applied or discarded), the edit is refused
  with 409, and the browser shows the new revision first.
- `value: null` drops vislox's claim: lox puts back the value Loxone Config had before.
- The reply has, per op, what undoes it (`previous`), the new pending blob id, and the blocks it added with the
  UUIDs lox minted (`added: [{ slug, uuid }]`). For a value, `previous` is the value vislox claimed before: the
  op with that value undoes it. For `remove_block` it is the `add_block` that adds the block again; for
  `add_block` it is null, and `remove_block` with the new UUID undoes it. For `move` it is the `move` back to
  where the block was; for `relayout`, the list of `move`s that put each block back. That is all #62 needs from
  the server.
  The history itself can live in the browser.
- All ops of one request are checked first, then written and compiled once. If anything fails, or the request
  goes away mid-compile (a closed tab), the file, the lock and `out.Loxone` are put back as they were.
- **An edit never takes along someone else's uncompiled changes.** The compile covers the whole module, so
  before it, vislox checks that the module is what `out.Loxone` was compiled from. When a module file is newer
  than `out`, it compiles the module as it is into a scratch copy (copied lock, `--out` elsewhere) and compares;
  a different result refuses the edit with 409, naming the files: compile or discard them first.

Later steps add operations to the same list: `wire` and `unwire` (#61).

## How an op becomes Lox Logic

vislox owns the objects it edits through Lox Logic claims on an `extern`, pinned by UUID:

```text
extern vx_10000006_0000_0060_ffff504f94000001 = Div(uuid: "10000006-0000-0060-ffff504f94000001")
vx_10000006_0000_0060_ffff504f94000001.title = "Teiler"
vx_10000006_0000_0060_ffff504f94000001.In2 = 100
```

lox records each original value in the lockfile and restores it when the line goes. vislox reads the file back
with lox's parser, refuses one that holds anything it did not write, and rewrites it whole on every edit.

A new block (#59) is a managed block, declared in vislox's file for its page:

```text
# Blocks added in vislox on this page. vislox rewrites this file on every edit;
# see docs/editing.md in vislox.
# page uuid: 10000008-0000-0080-ffff504f94000001
page "Obergeschoss"

vx_monoflop_1 = Monoflop("Nachlauf", Time: 30)
vx_pulseat_1 = PulseAt(At: 07:05, Pulse: "time")
```

- lox mints the block's UUIDs into the lockfile and places it on the page; vislox reads the UUID back from the
  lockfile after the compile.
- vislox picks the slug, `vx_<type>_<n>`, the lowest `n` that no name of the module, no lockfile entry and no
  removal tombstone has. It never follows the name, so renaming a block never changes its identity.
- `page "…"` names the page by its title, and lox builds on the first page with that title. So a block is added
  only to a page whose title no earlier page of the project's base has.
- The file belongs to its page by UUID (the `# page uuid:` line), not by title. When Loxone Config renames the
  page, the next edit writes the new title into the same file. A file written without the line takes the first
  page with its title.
- The file is named after the page's title when it is made (`Küche Licht` → `kueche-licht.loxl`); one with that
  name for another page gets `-2`. It is deleted when its last block and `removed` line are gone.
- A block added here is edited in Properties like any other: its name, its unwired inputs' constants and its
  settings (`Formula:`, `Type:`, `At:`, …) change its declaration, not a claim. lox has no room or category for
  a managed block, so the form does not offer them. An empty setting drops it: Loxone Config's value again.
- **Remove** takes out a block added here that is not deployed yet: not in the deployed revision, the same one
  the browser marks *not deployed* (the newest commit of the base). Its declaration goes, and
  `removed vx_monoflop_1` tells lox the lockfile entry is meant to go (lox refuses a managed block the source
  lost). vislox drops that line once the lockfile no longer has the block. A deployed block is deleted with #80.
- **Discard** puts the module, lock and out back to the last commit. A lock or out the last commit does not have
  (never committed, or ignored by git) goes back to how it was before vislox's first edit since that commit,
  which vislox keeps in the git directory; one vislox never edited stays as it is, since a lost lock re-mints
  every managed block. Discard is all or nothing: when a step fails, it puts back what the others did.

## Moving blocks (#60)

Lox Logic has no statement for a position. lox keeps the rectangle of each block it manages in the lockfile
(`objects.<slug>.layout`, with `page_uuid`), places a new block once, and from then on keeps it, except that a
move made in Loxone Config is taken over from the base. `lox logic compile --relayout <page>` places a page's
managed blocks afresh. vislox moves blocks with the same lockfile state lox's own placement leaves:

- `move` sets the block's `layout` to the new rectangle: the top left corner snapped to Loxone Config's grid
  (multiples of 96, as a Loxone Config save writes them), the size kept. When the base still draws the block
  elsewhere on its page, that rectangle goes into `superseded_layout`, the field lox uses to tell a base from
  before its placement from a move made in Loxone Config. Without it, the compile would take the base's position
  back. Once applied, the base draws the block at its new place and lox drops the field itself.
- After the compile, vislox checks that the pending config draws each moved block where it was put; a move lox
  did not keep is refused, and the lock and out are put back.
- `relayout` runs `lox logic compile --relayout <page title>`. lox lays out every page with that title, so a page
  whose title another page has is refused, as is a page with no managed block.
- What is moved: the blocks the lockfile manages that are drawn on a page. That is every block added here (#59),
  the module's own blocks, and, once adopted (#72), a block Loxone Config drew. Their values are still edited
  only where they are declared; the position is not in the source, so moving them changes no `.loxl` file.
- Wires: Loxone Config draws a wire from its two ends, and so does vislox, so a wire follows its blocks. While a
  block is dragged, its wires are drawn straight with one bend; the compile's page draws them routed again.
- A move is in the pending revision like any edit: the Changes list and the canvas show the block *moved*, with
  the deployed position as a dashed ghost. Discard puts the lock back with the rest. Apply deploys it: `lox config
  push` compiles with the lockfile, so the move goes along.

vislox offers only what lox (v0.83.0, the version `Cargo.lock` and the Docker image pin) can express. What lox
cannot express is left out, not worked around.

| Step | Lox Logic | Notes |
|---|---|---|
| #58 name | `x.title = "…"` | Not on references: Loxone Config takes their title from what they mirror. |
| #58 room, category | `x.room = "…"`, `x.category = "…"` | By title, which must name one room or category. Only on an object that has one: lox changes it, never adds it. |
| #58 input constant | `x.Port = n` | Only an unwired input; a number. lox checks the documented range. |
| #58 setting | `x.MinTime = n`, `x.Desc = "…"` | Physical inputs and outputs, Tree and Air devices and extensions. Most settings can only be changed, not added. |
| #59 new block | `slug = Type("Title", Port: n, Setting: …)` after `page "…"` | One file per page, `<module>/vislox/<page>.loxl`; lox mints the UUIDs and places it. |
| #59 remove a new block | drop it, `removed slug` | Only one not deployed yet. |
| #60 move | none: `layout` in the lockfile | Only a block the module manages; snapped to the grid. |
| #60 arrange | `lox logic compile --relayout <page>` | Lays out the page's managed blocks afresh. |
| #61 wire | `x.Port <- y.Q` | Adds a wire, and removes one vislox added. A wire Loxone Config drew cannot be removed. |

Blocks the module already manages (they are in the lockfile) are refused: they are edited in their own `.loxl`.
The blocks vislox added are the exception: their `.loxl` is vislox's.

## What lox cannot do, so vislox does not offer it

- Parameters stored on the block element of a block the module does not manage: a Formula's text, a Daytimer's
  On/Off labels, a memory flag's type, a virtual input's Analog, MinVal, MaxVal and DefVal, and `Desc` on logic
  blocks. lox takes them only in the declaration of a block it manages: adopt the block first (#72), then edit
  it as a managed block (#79).
- A room or category on an object that has none.
- Handing a value back after Apply: deleting a claim makes lox restore the value from before the claim, which
  undoes the applied change. So once applied, the module owns the value, and a later change in Loxone Config is
  overwritten on the next Apply. Revert before Apply works.
- Moving an object it does not manage: a block Loxone Config drew, an input or output reference (`InputRef`,
  `OutputRef`), a note, a comment. lox never writes their position; they are moved in Loxone Config, or adopted
  first (#72). Arranging lays out only managed blocks, and leaves one wired to a reference the module does not
  manage beside that reference.
- Removing a wire Loxone Config drew.
- New blocks of some types. vislox offers the page blocks of lox's verified builtin table (`loxl types`) that
  lox creates and that work as placed; a test compiles each one with lox. Not offered:
  - virtual inputs, virtual HTTP, UDP and text inputs, virtual outputs and their commands, Modbus servers,
    devices and their I/O, and operating modes: objects of the object tree, not of a page (#84);
  - the NFC Code Touch, Intercom and Touch Pure Flex controller blocks, which exist because Loxone Config paired
    the device (lox adopts them, never creates them);
  - blocks lox creates but that do nothing until they are set up in Loxone Config: the burglar and smoke alarms,
    the room controller, the irrigation, and the central light and shade blocks (their members are #75);
  - every type outside lox's table (lox refuses it: "Refuse, never guess"), and templates (#83).
- On a new block: a program's text (`Code:` takes only `file(…)`), a wire or an expression on an input (#61, #82),
  a position when it is added (lox places it; it can be moved after), and a room or category (lox has no claim for them on a managed block). Its settings
  (`Formula:`, `Type:`, `At:`, …) are given when it is added; changing them later is #79.
- A block on a page whose title an earlier page has, and a new page (#88).

## The rest of the language

Each of these plugs in as new op kinds on `POST /api/edit`, written into `<module>/vislox/`:

| Issue | Lox Logic |
|---|---|
| #59 (built) | new blocks (`slug = Type(…)`, `page`), `removed` for one not deployed |
| #60 (built) | positions of managed blocks (the lockfile's `layout`), `--relayout` |
| #61 | wires (`<-`) |
| #62 | undo and redo |
| #72 | adopt a block into the module |
| #73 | Daytimer switching times (`schedule`, `.schedule`) |
| #74 | lighting moods and circuits (`moods`, `.moods`) |
| #75 | members of a central block (`.members`) |
| #76 | NFC Code Touch access (`access`, `.access`) |
| #77 | notification recipients and messages (`recipients`, `OutputRefLM`) |
| #78 | Touch Pure Flex tiles (`tiles`, `.tiles`) |
| #79 | a managed block's label and settings (`Desc:`, `Type:`, `Formula:`, `Code: file(…)`, …) |
| #80 | delete and rename managed blocks (`removed`, `moved`) |
| #81 | `let` and `defaults` |
| #82 | expressions and named signals |
| #83 | `template` instances |
| #84 | virtual inputs, virtual and Modbus devices |
| #85 | Tree and Air hardware (`Statistics:`, new Tree devices) |
| #86 | operating modes and calendar entries |
| #87 | Automatic Designer rules (`rule`) |
| #88 | new pages (`page "…" (room: …)`) |

Around the module: #64 (mark managed blocks), #65 (`lox logic check` errors), #66 (lint), #67 (`lox logic test`),
#68 (review claimed access, recipients and schedules), #70 (removals on Apply).

## The browser

- Properties has an *Edit* section on the newest revision: *Edit values* opens a form for the name, the
  room and category, the unwired inputs and the settings. Numbers are checked in the browser; lox checks ranges and the rest.
- *Add block* (top left of the page, on the newest revision) lists the types vislox offers (`GET /api/block-types`:
  each type's purpose and link to Loxone's documentation, its ports with their units, canvas labels and the value
  Loxone Config places them with, and its settings), searched by name, type and purpose. The form asks for the
  name, the settings, the parameters and, folded, the inputs' constants; an empty field keeps Loxone Config's
  value. After the compile the new block is selected on its page, marked *added here, not deployed*, with
  *Remove*. The Deploy pane counts the blocks added here.
- **Not deployed** means the pending config's value differs from the deployed config's, for a value vislox
  claims now or claimed in the deployed revision. The deployed revision is the newest git commit of the base
  (the last Apply, Pull or Sync), as for the app's calendar entries, so a plain `git commit` deploys nothing.
  Such a value shows *not deployed*, with *Revert*: back to what the deployed revision claimed (or no claim).
  The Deploy pane counts them.
- A block the module manages is moved by dragging it on the newest revision (the cursor shows which can be), or
  with Alt and an arrow key when it is selected (Shift: ten grid steps). It snaps to the grid, its wires follow,
  and the drop is compiled. Properties shows *moved, not deployed* with *Revert* (back to the deployed position),
  and the Deploy pane counts the blocks moved. Dragging any other block pans the page, as before.
- *Arrange*, next to *Add block*, lays out the page's managed blocks afresh. It is off on a page without them.
- `GET /api/edits` lists what vislox's files claim now and in the deployed revision, which of it is not
  deployed, the blocks vislox added (`blocks`: slug, UUID, type, page, file, and whether the deployed config has
  it), the other blocks the module manages, the blocks lox can move (`movable`: managed and drawn on a page) and
  those the pending config draws elsewhere than the deployed one (`moves`: UUID, position, deployed position). It
  is worked out again only when vislox's files, the deployed or
  the pending config changes (`actions.edit.key`); the browser fetches it only then.
