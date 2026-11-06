# Editing in vislox

vislox becomes an editor in steps: edit values (#58), add blocks (#59), move blocks, inputs and outputs (#60),
make and change connections (#61), and undo and redo (#62). This page is the design they share. #58 is built;
the rest says where each step plugs in.

## The rules

- **Only a `lox.toml` project is edited.** It is the only source with a pending revision, Apply and Discard.
  A `.Loxone` file, a backup or a `lox config init` repository stays read-only.
- **Edits go into the module, never into a `.Loxone`.** `lox config push` compiles the module again before it
  uploads, so a change made only in `out.Loxone` would be lost on Apply, and `base.Loxone` is what the
  Miniserver runs.
- **vislox writes one place: its own files under the module directory**, today `<module>/vislox/edits.loxl`.
  The rest of the module is never touched. A module that is one file cannot be edited from vislox.
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
           { "op": "setting", "uuid": "…", "name": "MinTime", "value": "500" } ] }
```

- `base` is the git blob id of what the browser showed: the newest compile, or the deployed config when nothing
  is pending. When the file on disk is another one (someone compiled, applied or discarded), the edit is refused
  with 409, and the browser shows the new revision first.
- `value: null` drops vislox's claim: lox puts back the value Loxone Config had before.
- The reply has, per op, the value vislox claimed before (`previous`), and the new pending blob id. An op with its
  `previous` value undoes it: that is all #62 needs from the server. The history itself can live in the browser.
- All ops of one request are checked first, then written and compiled once. If anything fails, or the request
  goes away mid-compile (a closed tab), the file, the lock and `out.Loxone` are put back as they were.
- **An edit never takes along someone else's uncompiled changes.** The compile covers the whole module, so
  before it, vislox checks that the module is what `out.Loxone` was compiled from. When a module file is newer
  than `out`, it compiles the module as it is into a scratch copy (copied lock, `--out` elsewhere) and compares;
  a different result refuses the edit with 409, naming the files: compile or discard them first.

Later steps add operations to the same list: `add_block` (#59), `move` (#60), `wire` and `unwire` (#61).

## How an op becomes Lox Logic

vislox owns the objects it edits through Lox Logic claims on an `extern`, pinned by UUID:

```text
extern vx_10000006_0000_0060_ffff504f94000001 = Div(uuid: "10000006-0000-0060-ffff504f94000001")
vx_10000006_0000_0060_ffff504f94000001.title = "Teiler"
vx_10000006_0000_0060_ffff504f94000001.In2 = 100
```

lox records each original value in the lockfile and restores it when the line goes. vislox reads the file back
with lox's parser, refuses one that holds anything it did not write, and rewrites it whole on every edit.

vislox offers only what lox (v0.83.0, the version `Cargo.lock` and the Docker image pin) can express. What lox
cannot express is left out, not worked around.

| Step | Lox Logic | Notes |
|---|---|---|
| #58 name | `x.title = "…"` | Not on references: Loxone Config takes their title from what they mirror. |
| #58 room, category | `x.room = "…"`, `x.category = "…"` | By title, which must name one room or category. Only on an object that has one: lox changes it, never adds it. |
| #58 input constant | `x.Port = n` | Only an unwired input; a number. lox checks the documented range. |
| #58 setting | `x.MinTime = n`, `x.Desc = "…"` | Physical inputs and outputs, Tree and Air devices and extensions. Most settings can only be changed, not added. |
| #59 new block | `slug = Type("Title", Port: …)` after `page "…"` | One file per page, `<module>/vislox/<page>.loxl`; lox mints the UUIDs. |
| #60 move | none | lox places the blocks it creates; it has no claim for a position. |
| #61 wire | `x.Port <- y.Q` | Adds a wire, and removes one vislox added. A wire Loxone Config drew cannot be removed. |

Blocks the module already manages (they are in the lockfile) are refused: they are edited in their own `.loxl`.

## What lox cannot do, so vislox does not offer it

- Parameters stored on the block element of a block the module does not manage: a Formula's text, a Daytimer's
  On/Off labels, a memory flag's type, a virtual input's Analog, MinVal, MaxVal and DefVal, and `Desc` on logic
  blocks. lox takes them only in the declaration of a block it manages: adopt the block first (#72), then edit
  it as a managed block (#79).
- A room or category on an object that has none.
- Handing a value back after Apply: deleting a claim makes lox restore the value from before the claim, which
  undoes the applied change. So once applied, the module owns the value, and a later change in Loxone Config is
  overwritten on the next Apply. Revert before Apply works.
- Moving a block it does not manage, and removing a wire Loxone Config drew.

## The rest of the language

Each of these plugs in as new op kinds on `POST /api/edit`, written into `<module>/vislox/`:

| Issue | Lox Logic |
|---|---|
| #59 | new blocks (`slug = Type(…)`, `page`) |
| #60, #61 | move; wires (`<-`) |
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
- **Not deployed** means the pending config's value differs from the deployed config's, for a value vislox
  claims now or claimed in the deployed revision. The deployed revision is the newest git commit of the base
  (the last Apply, Pull or Sync), as for the app's calendar entries, so a plain `git commit` deploys nothing.
  Such a value shows *not deployed*, with *Revert*: back to what the deployed revision claimed (or no claim).
  The Deploy pane counts them.
- `GET /api/edits` lists what vislox's file claims now and in the deployed revision, which of it is not
  deployed, and the blocks the module manages. It is worked out again only when vislox's file, the deployed or
  the pending config changes (`actions.edit.key`); the browser fetches it only then.
