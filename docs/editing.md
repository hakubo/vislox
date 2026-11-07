# Editing in vislox

vislox becomes an editor in steps: edit values (#58), add blocks (#59), move blocks, inputs and outputs (#60),
make and change connections (#61), undo and redo (#62), adopt the blocks Loxone Config drew (#72), edit a
Daytimer's switching times (#73), a lighting controller's moods (#74) and a central block's members (#75). This
page is the design they share. #58 to #62, #72 to #75 are built; the rest says where each step plugs in.

## The rules

- **Only a `lox.toml` project is edited.** It is the only source with a pending revision, Apply and Discard.
  A `.Loxone` file, a backup or a `lox config init` repository stays read-only.
- **Edits go into the module, never into a `.Loxone`.** `lox config push` compiles the module again before it
  uploads, so a change made only in `out.Loxone` would be lost on Apply, and `base.Loxone` is what the
  Miniserver runs.
- **vislox writes one place: its own files under the module directory**, `<module>/vislox/`: `edits.loxl` for
  the values it claims and the wires it draws, and one file per page for the blocks it adds
  (`<module>/vislox/obergeschoss.loxl`).
  The rest of the module is never touched. A module that is one file cannot be edited from vislox.
  The exceptions are the lockfile: a move (#60), since lox keeps a managed block's position there, not in the
  source, so vislox writes it there the way lox's own `--relayout` does (see *Moving blocks*); and an adopted
  block's entry (#72), which `lox logic adopt` writes on a copy and vislox copies over (see *Adopting blocks*).
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
           { "op": "relayout", "page": "<page uuid>" },
           { "op": "place", "moves": [ { "uuid": "…", "x": 1920, "y": 960 } ] },
           { "op": "wire",   "from": ["<uuid>", "AQ"], "to": ["<uuid>", "I2"] },
           { "op": "unwire", "from": ["<uuid>", "Q"],  "to": ["<uuid>", "I1"] },
           { "op": "adopt", "blocks": [ { "uuid": "…", "slug": "vx_licht" } ] },
           { "op": "adopt_page", "page": "<page uuid>" },
           { "op": "unadopt", "uuids": [ "…" ] },
           { "op": "schedule", "uuid": "…", "value": { "modes": { "<mode uuid>": [
               { "from": 390, "to": 480, "value": "21.5", "activate": true } ] } } },
           { "op": "moods", "uuid": "…", "value": { "moods": [
               { "name": "Viel Licht", "values": "all on" },
               { "name": "Lesen", "values": "AQ1 on, AQ2 40%" } ] } },
           { "op": "members", "uuid": "…", "value": [ "<member uuid>", "<member uuid>" ] } ] }
```

`adopt`, `adopt_page` and `unadopt` go in a request of their own (#72).

- `base` is the git blob id of what the browser showed: the newest compile, or the deployed config when nothing
  is pending. When the file on disk is another one (someone compiled, applied or discarded), the edit is refused
  with 409, and the browser shows the new revision first.
- `value: null` drops vislox's claim: lox puts back the value Loxone Config had before.
- The reply has, per op, what undoes it (`previous`), the new pending blob id, and the blocks it added with the
  UUIDs lox minted (`added: [{ slug, uuid }]`). For a value, `previous` is the value vislox claimed before: the
  op with that value undoes it. For `remove_block` it is the `add_block` that adds the block again; for
  `add_block` it is null, and `remove_block` with the new UUID undoes it. For `move` it is the `move` back to
  where the block was; for `relayout`, the `place` that puts each block back (one op however many
  blocks the page has; `place` moves several managed blocks as `move` does), and for `place`, the `place` back. For `wire` it is the
  `unwire` of the same ends, and for `unwire` the `wire`. For `adopt` and `adopt_page` it is the `unadopt` of
  the blocks adopted, and for `unadopt` the `adopt` with the same slugs: a block keeps its UUIDs both ways. For
  `schedule` it is the `schedule` of what the timer claimed before: its set and that set's switching times, or
  no claim; when the op rewrote a set other timers still claim, it also carries that set's old switching times
  (`restore`), so the undo gives them back too. `moods` is the same for a controller's moods (#74). For
  `members` it is the list vislox claimed before, or null (#75).
  A reconnected wire is one request, `unwire` then `wire`:
  undone, the reverse list. The ops of one request see each other: `unwire` then `wire` of the same wire is
  accepted (and changes nothing). `remove_block`'s `add_block` carries the wires the removal took out
  (`wires`, an empty UUID for the block itself), since lox mints new UUIDs when it is added again, and names the
  block it adds again (`restores`: its old UUID, which changes nothing in what is written; the reply's `added`
  echoes it next to the new UUID).
- The reply also has `undo`: the ops that undo the whole request, built from `previous` (the last op's first; a
  value op with the value it replaced; `add_block` as `remove_block` of the UUID lox minted; a `relayout` as its
  moves, one `place`). It has one op per op of the request, so it never holds more than a request takes. When it
  cannot be built (lox named no UUID for a block the request added), it is null and `undo_error` says why. A
  `previous` that is not a value is always an op, never null. See *Undo and redo*.
- All ops of one request are checked first, then written and compiled once. If anything fails, or the request
  goes away mid-compile (a closed tab), the file, the lock and `out.Loxone` are put back as they were.
- **An edit never takes along someone else's uncompiled changes.** The compile covers the whole module, so
  before it, vislox checks that the module is what `out.Loxone` was compiled from, by content, not by file times
  (two writes can share one). A module and `out` exactly as vislox's last compile left them (each file's blob id,
  and out's) pass; otherwise vislox compiles the module as it is into a scratch copy (copied lock, `--out`
  elsewhere) and compares. A different result refuses the edit with 409, naming the files that changed: compile
  or discard them first.

Later steps add operations to the same list.

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

## Adopting blocks (#72)

A block drawn in Loxone Config is edited from source only once the module manages it, and lox refuses some of
its values on a block it does not manage (a Formula's text, a Daytimer's On/Off, a memory flag's type, `Desc` on
logic blocks). `lox logic adopt <base> --uuid <uuid> --as <slug> --module … --lock …` puts it under the module:
the block's declaration, externs for its wired neighbours and `<-` wires onto them, appended to the module's file
of its page, and its UUIDs, layout and page pinned in the lockfile, written only once lox has rebuilt the config
unchanged in memory. lox has no structured edit command, so vislox works on what adopt writes:

- **On a copy.** vislox copies the module and the lockfile into a scratch directory and runs `lox logic adopt`
  there, from the project, against the project's base (lox adopts from the config it compiles against, so the
  block must be in the base). It never runs it on the project: lox would write the module's own files.
- **Into vislox's files.** What lox appended (the statements after each file's old ones; a new file's `page` line
  aside) is moved into vislox's file of the page (`<module>/vislox/<page>.loxl`, the same file blocks added there
  live in), under a `# Adopted from Loxone Config` comment: the declaration, then its wires. Every extern lox
  pinned (by title or iname) becomes vislox's extern of that object, `vx_<uuid>` pinned by UUID in `edits.loxl`,
  and every reference is renamed to it, so vislox's files never name the module's own slugs (a bare name that is
  a `let` constant of the module or a day word, `Days: mon`, stays a value: lox reads it so first); such an extern stays
  while an adopted block names it. A neighbour the rest of the module manages is refused (vislox's files would
  name its slug); a neighbour of vislox's files is named by its slug.
- **The lockfile** gains the block's entry (`objects.<slug>`, as lox wrote it on the copy, keyed by vislox's slug)
  and the `extern_wires` lox recorded for its wires onto externs; lox's own extern entries stay on the copy (the
  compile pins vislox's by UUID). Then `lox logic compile` runs as for any edit.
- **Checked.** After the compile, the pending config must be what it was (`loxone_config::diff`: no block, value,
  place or wire changed) and each adopted block exactly as the base has it: its element (UUIDs, connectors and
  their wires, parameters, place; attribute order and whitespace aside) and the wires out of it. Anything else
  refuses the edit, and the files, lock and out are put back.
- **The slug** is `vx_` and lowercase letters, digits and `_` (vislox's files hold only `vx_` names), not taken by
  the module, its lockfile (tombstones too) or vislox's files; left empty, vislox makes one from the block's name
  (`Licht Küche` → `vx_licht_kueche`), else its type.
- **Not offered** (`adopt` in `GET /api/edits`: UUID → null, or why): a block not on a logic page (devices and
  the object tree are #84 and #85), of a type lox has no table for (notes, …), not in the base, on another page
  than in the base, with values or wires vislox's edits claim (revert them first: lox would move those lines into
  the declaration), an extern of the module's own files (lox adopts it only under that name and rewrites those
  files). lox's own refusals (a block version or attribute its rebuild does not know, a wire into a
  managed block its declaration does not bind) come back as lox says them; so does any warning, and the block
  stays Loxone Config's. A block whose program lox would write into a file of its own (`Code: file(…)`) is
  refused too.
- **`adopt_page`** adopts every block of the page that is offered, in the order that feeds lox the sources before
  the blocks they feed (lox adopts a block wired into a managed one only when that one's declaration binds the
  wire, and binds it when it adopts the sink second; a block an earlier adoption of the request pinned as an
  extern is adopted under that extern's name, and renamed after). A block that blocks adopted earlier wire to
  through vislox's extern of it (`vx_<uuid>`) is adopted the same way: lox moves their `<-` lines on the extern
  into its declaration, and they are the only lines that may leave vislox's files; their bindings naming the
  extern are renamed to its slug, and the `extern_wires` and `externals` entries lox dropped leave the lock. The
  reply's `skipped` names each block it left
  to Loxone Config and why; `adopted` names the slugs.
- **Managed then.** The block is in `blocks` (`adopted: true`, and `committed` once the deployed revision's commit
  has it) and is a block of vislox's files: its name, unwired inputs' constants and settings change its
  declaration, it is moved (#60), and wired (#61) like a block added here (lox ORs a second wire into a bound
  input). The wires its declaration brought along are its declaration's now (`adopted_wires`, `wire_why.adopted`):
  vislox does not take them out yet. Remove refuses it (it is deployed); deleting a managed block is #80.
- **`unadopt`** (the undo, and *Hand back* in Properties) takes the declaration and its wires out of vislox's file,
  the entry out of the lockfile with every `extern_wires` entry from its ports (without its line lox would take
  those wires out of the config: they are Loxone Config's again), and drops the externs nothing else names. The
  blocks that stay adopted keep every wire between them and it, onto vislox's extern of it: their bindings and
  `<-` lines name `vx_<uuid>`, and a wire into it that its own declaration bound from one of them becomes that
  block's `<-` line (from a block added here, a line of `edits.loxl`). Only a block whose pending state is the
  base's (what was changed here is reverted first), and that no wire drawn here ends at. It is checked like an
  adoption. `blocks[].hand_back` says why it cannot be handed back now (the same op, tried on a copy), and the
  button is off with that reason.
- **Nothing pending.** Adopting changes nothing the Miniserver runs, so `out` stays as it was and nothing is
  pending: Apply has nothing to deploy. The Deploy pane says how many blocks adopted here are not committed yet;
  they go into git with the next Apply. Undo hands them back; Discard (with something pending) drops them too.

Later: #79 edits the settings lox takes only on a managed block, #80 deletes and renames adopted blocks, and
taking out a wire an adopted declaration brought along is left to a later step.

## Moving blocks (#60)

Lox Logic has no statement for a position. lox keeps the rectangle of each block it manages in the lockfile
(`objects.<slug>.layout`, with `page_uuid`), places a new block once, and from then on keeps it, except that a
move made in Loxone Config is taken over from the base. `lox logic compile --relayout <page>` places a page's
managed blocks afresh. vislox moves blocks with the same lockfile state lox's own placement leaves:

- `move` sets the block's `layout` to the new rectangle: the top left corner snapped to Loxone Config's grid
  (multiples of 96, as a Loxone Config save writes them) and not past the page's top left (0, 0), the size kept.
  A target within a grid step of where the base draws the block is that rectangle as it is, on the grid or not:
  lox keeps the base's rectangle there, so *Revert* puts a block Loxone Config left off the grid back exactly. When the base still draws the block
  elsewhere on its page, that rectangle goes into `superseded_layout`, the field lox uses to tell a base from
  before its placement from a move made in Loxone Config. Without it, the compile would take the base's position
  back. Once applied, the base draws the block at its new place and lox drops the field itself.
- After the compile, vislox checks that the pending config draws each moved block where it was put (or, within
  a grid step of the base's rectangle, there); a move lox did not keep is refused, and the lock and out are put
  back. A move on a page the same request lays out afresh is not checked: lox places those blocks anew.
- `relayout` runs `lox logic compile --relayout <page title>`. lox lays out every page with that title, so a page
  whose title another page has is refused, as is a page with no managed block, and one whose blocks a lock from
  before lox pinned pages (no `page_uuid`) holds.
- What is moved: the blocks the lockfile manages that are drawn on a page. That is every block added here (#59),
  the module's own blocks, and, once adopted (#72), a block Loxone Config drew. Their values are still edited
  only where they are declared; the position is not in the source, so moving them changes no `.loxl` file.
- Wires: Loxone Config draws a wire from its two ends, and so does vislox, so a wire follows its blocks. While a
  block is dragged, its wires are drawn straight with one bend; the compile's page draws them routed again.
- A move is in the pending revision like any edit: the Changes list and the canvas show the block *moved*, with
  the deployed position as a dashed ghost. Discard puts the lock back with the rest. Apply deploys it: `lox config
  push` compiles with the lockfile, so the move goes along.

## Wires (#61)

A wire is a Lox Logic `<-` statement, `sink.Port <- source.Port`, written into `edits.loxl` after the claims:

```text
extern vx_10000004_0000_0040_ffff504f94000001 = And(uuid: "10000004-0000-0040-ffff504f94000001")
extern vx_10000006_0000_0060_ffff504f94000001 = Div(uuid: "10000006-0000-0060-ffff504f94000001")

# Wires drawn in vislox. Deleting a line takes the wire out again.
vx_10000004_0000_0040_ffff504f94000001.In2 <- vx_10000006_0000_0060_ffff504f94000001.AQ
vx_and_1.In1 <- vx_10000006_0000_0060_ffff504f94000001.AQ
```

- `wire` names its ends as the page does: `[object UUID, connector key]`, from an output to an input. vislox
  writes the ports by their Lox Logic names (`In2` for `I2`).
- An end Loxone Config drew is an `extern` pinned by UUID, the same one a claimed value uses; one that only a
  wire needs stays as long as the wire. An end vislox added (#59) is named by its slug: on a block declared in the
  module, lox folds `<-` into the block's arguments, as if it were written there. A block the rest of the module
  manages is refused, as for values: it is wired in its own `.loxl` (lox could take the wire here, but vislox's
  file would then name the module's own slug, and a rename there would break it).
- lox records each wire it draws onto an extern in the lockfile (`extern_wires`) and takes it out again when the
  statement goes, leaving the wires Loxone Config drew alone. So `unwire` takes out a wire vislox's file draws,
  deployed or not, and refuses one Loxone Config drew ("remove it in Loxone Config") or the rest of the module draws.
- An input may take several wires (Loxone Config ORs or ANDs them, by block). A wire onto an input that already has
  one adds a second; reconnecting is `unwire` then `wire`, so only a wire vislox drew can be reconnected.
- Checked before lox sees it, because lox does not check a wire's direction on an extern: the source is an output
  and the sink an input or parameter (by lox's port table, else by the side Loxone Config draws it on), on two
  different objects; the pair is not wired yet; the sink is not inverted in Loxone Config (`Inv=`, which lox leaves
  to Loxone Config); and an input with a constant edited here is cleared first (lox refuses a constant and a wire
  on one port of a declaration). Both ends must be in the base lox compiles against.
- Types: Loxone Config wires digital and analog connectors to each other (a value other than 0 is on), but text
  only to text, and lox does not check it on an extern. vislox takes what a connector carries from what is known:
  a reference's `Text="true"` or `Analog="true"`; the value of the objects lox mints references for (virtual
  inputs, sensors and actuators, a memory flag by its `Tp=`); the text connectors of lox's port table (`TQ…`,
  `OutText`, `Text`, `inText`, `TTS`, `TM`, `TI`); and any other connector of lox's port table as analog by its key
  (`AQ`, `AI`, …) or digital. A connector none of these knows is *unknown*, and a wire to it is refused rather than
  guessed. The page sends each port's kind (`kind` on a port) so the browser marks the targets while dragging; the
  server checks again.
- Who draws a wire vislox's file does not: the rest of the module, for a wire lox tracks onto an extern
  (`extern_wires`) or one into a block the module declares, else Loxone Config. `unwire` refuses by this rule, and
  `GET /api/edits` sends the module's wires (`module_wires`) and both reasons (`wire_why`) so the card says the
  same.
- Taking out a block added here takes its wires with it; its undo (`previous`) draws them again.
- A wire in the file whose end is neither an extern of `edits.loxl` nor a block vislox added (its line deleted by
  hand) is refused when the files are read, naming the wire: it would break every compile.
- In the pending revision the Changes list and the canvas show a wire added (green) or removed (red, dashed), as
  for any compile. Discard drops them with the rest; Apply deploys them (`lox config push` compiles the module).

## Blocks the module manages (#64)

The canvas marks the blocks the module manages, so a reviewer knows whether a block is changed in a `.loxl` file
or in Loxone Config (where an edit to a managed block is overwritten on the next push).

- **Managed** means the lockfile pins the block's UUID: an entry of its `objects` (slug → UUID). The `externals`,
  which `extern` names, are Loxone Config's and stay unmarked. The blocks added here (#59) are managed too.
- `GET /api/edits` sends `sources`: UUID → `{ slug, file, line, how }`, `file` relative to the project and `line`
  1-based, relative to the project (to the module when it lies outside it; never absolute). vislox reads them off
  the module's `.loxl` files with lox's own declaration scan (the one its language server lists symbols with), at
  the top level only: what a `template … end` body declares is made once per instance, so it is never a block's
  line. `lox logic check --json` gives only errors' lines, and lox has no command that maps a slug to its line.
  - `declared`: a block, named signal or template instance declared by that name.
  - `expression`: a block lox made for an expression, named `<sink>_<port key>__<op><n>` (lox's `desugar.rs`), at
    the line of the expression: the `<-` statement, the argument of the block it feeds, or the named signal
    (`<signal>__<op><n>`). Two expressions into one port share a prefix; the first one's line is given.
  - `template`: a block a template instance made (`<instance>_<block>`), at the instance's line (pointing into the
    template's body is left to #83).
  - `ambiguous`: a name declared twice at the top level (lox refuses that module), or an expression into such a
    name: no line, rather than a guess.
  - none (`file` null): the lockfile pins it, but no `.loxl` file declares it now.
- Like the rest of `/api/edits`, it is worked out again only when its key changes; the key covers the module's
  `.loxl` files, so a line moved in the module and not compiled yet is followed within the next refresh (3 s).
  Each refresh looks at the files' sizes and modification times, and reads and hashes only those that changed, or
  whose time is under 3 s old (two writes in one tick may share it); `sources` uses the texts it read.
- They are the lockfile and module as they are now, so the browser shows them on the newest config only (the
  pending compile, else the deployed config on disk), not on an older revision of the History.
- The browser draws a λ badge on each managed block, a *Lox Logic* section in Properties (slug, `file:line`,
  *Copy path*), a *managed* key and *Show managed blocks only* in the legend (only when the project has managed
  blocks), and finds a block by its slug in Ctrl+K. Values of a managed block are still refused in *Edit values*,
  naming its `file:line`.

## When the module does not compile (#65)

vislox shows the compiled `out.Loxone` as the pending revision. When the module no longer compiles (a typo, an
unknown `extern` title, a block type whose ports lox has not verified), `out` stays at the last compile that worked,
and the review would show an older change than the one in the source. So vislox checks the module itself.

- **The check worker** runs `lox logic check --json <module>` and `lox logic fmt --check <module>` (at once) in the
  project whenever something it depends on changes: the module's files and the lockfile (the edit key), and the
  base and out by blob id (as the history reads them on every poll). A file change wakes it, after 300 ms for the
  writer to finish; so does the end of any action. `GET /api/check` only reads the last check it kept (`current`:
  nothing changed since); a request never runs lox. `actions.edit.check` counts the checks kept, and the browser
  fetches again when it moves.
- **Kept under the files lox read.** The key's module and lock part is taken when the watcher or the poll looks.
  A file changed since (and maybe changed back before the next look) would keep what lox said of the change under
  the key of the files after it. So the worker looks at the module and lock itself before and after lox runs, and
  keeps the check only when both looks are the key's. When the key lags the files, the worker brings its module
  and lock part up to date itself and checks again, rather than waiting for the next look. Its own look rereads
  and rehashes only a file (or the lock) whose size or time changed, off the async worker.
- The worker never checks while an action (an edit, Compile, Apply, Sync, Discard) runs: that check would be of a
  project half written. It keeps nothing then, and the action's end wakes it, so a check skipped during an Apply
  that failed and changed no file comes back as it was.
- **One compile at a time**: every action holds a compile lock while it runs (after `busy`), and the worker holds
  it for its check. An action that starts during a check waits for it; the worker leaves a running action alone.
- The module's directory is watched recursively, and its parent too, so a module directory made later (by the
  first edit) or made again is watched from then on. The 3 s poll still covers the rest.
- `errors`: each of lox's errors (at most 50; `truncated` counts the rest) with `file` (shown like `sources`:
  relative to the project), `line`, `message` (without lox's `compile error:` prefix) and `blocks`: the UUIDs
  the line makes or names. Those are the blocks `sources` (#64) puts on that line, and the names the line
  declares (`slug = …`, `extern slug = …`, `removed slug`) or references (the head of `slug.Port`), by the
  lockfile's slugs or vislox's `vx_<uuid>` names, and an extern's `uuid: "…"`. Strings, comments, types,
  keywords, argument keys and port names name nothing.
- `stale`: the module compiles, but not to the pending config: `{ files, error? }`. By content only: what vislox
  last compiled (an edit or *Compile*) is kept as the blob ids of each module file, the base and the out it wrote;
  when all match, out is the module compiled. Anything else is compiled into a scratch copy (copied lock, `--out`
  elsewhere) and compared with out. So a deleted file, a copy that keeps an old modification time and a new base
  are all seen. `files` are the files changed since vislox's last compile (the base among them), else those
  written no earlier than out (the message only). `error` is lox's message when the module checks but does not
  compile against the base; *Compile* is off then.
- `unformatted`: the files `lox logic fmt --check` names, on standard output or error. A hint only.
- `available: false` (with `reason`) when lox gives no answer vislox can read (an older lox, a stand-in): then
  nothing is shown and nothing is refused.
- **Apply's first step is *Module compiles***: it asks `lox logic check` anew (not the cache, which may be a
  refresh old), shown as it runs. With errors it fails there, before anything is sent, and Apply answers 409 with
  the first five errors. The browser turns Apply off with the reason. An edit is already refused then (its module
  check compiles the module).
- **Compile** (`POST /api/compile`, admin token): runs `lox logic compile` in the project, as an edit does, without
  changing the module, and records what it compiled. It snapshots the lock and out for Discard first, like an
  edit. It never deploys. lox's message comes back when it fails (a newer Loxone Config needs `--accept-version`,
  which Apply offers; Compile does not yet).
- When stale, Apply's confirmation says that `lox config push` compiles the module as it is, so Apply would
  deploy changes the review does not show. Apply is not refused for it.
- Discard forgets what vislox last compiled: after it, the module and out are the last commit's.

The Deploy pane shows it above Apply: *The module doesn't compile: N errors* with each error's `file:line`,
message and a button per block its line names (on the newest config); *Pending is from before the latest edit to
`pages/x.loxl`* (or, with nothing pending, *The module changed … and is not compiled yet*) with *Compile*; and the
formatting hint. Properties shows a block's errors in its *Lox Logic* section.

`lox logic test` results (#67) join this as lint does (below): `GET /api/check`'s `test`, worked out on the
same key, shown in the same pane and on the blocks the tests check.

## Lint findings (#66)

`lox logic lint` reports what compiles but is likely a mistake: unused externs, constants and templates, managed
blocks whose outputs feed nothing in the compiled config, Touch Pure Flex commands that reach several blocks or
none, users whose NFC tag or code opens no Code Touch, values outside a port's range, bare times that read better
with a unit, and more. vislox shows them; it never runs `--fix`.

- **Same check, same key, second step.** The check worker runs `lox logic lint .` in the project on the same key
  (module files, lock, base, out), so a request never runs it; but only after `check` and `fmt --check` came back
  without errors, and after that check is kept and told to the browsers. So errors never wait for lint (which
  compiles, and may take long). While the module does not compile, lint does not run (`lint: null`).
- **Lint gives way.** Lint compiles the module in memory for its dead-output analysis and writes nothing, so it
  does not hold the compile lock: an edit, Compile or Apply never waits for it. An action that starts while it
  runs stops it (the process is killed); the action's end wakes the worker, which lints the files as they are
  then. A lint whose files changed while it ran is not kept. Until lint is done, `lint` is `{ available: true,
  running: true, findings }` with the last lint's findings, so badges don't flicker on every change.
- **A lox without lint** (usage error: exit 2, `unknown command`) is found out once, by the lox binary's path,
  time and size: `lint` is `{ available: false, unsupported: true, reason }`, the browser says so in one quiet
  line, and lint is not asked again until that binary changes.
- **What lox prints.** lox has no JSON for lint (yet), so `check::parse_lint` reads its text: a header per kind
  (`dead-outputs (2)`), a line per finding (two spaces, then `` pages/a.loxl: `slug` — detail ``; the file only in a
  directory module), and the count it ends with (`lint: 3 findings`, or `lint: clean`). Any other line after a
  finding goes on with its detail, up to the next kind's header. Without that count the run failed
  (lox exits 1 for findings and for errors alike), and `lint` is `{ available: false, reason }` with lox's last
  lines.
- **Pinned like errors.** `check::locate_findings` gives each finding the line of its name's one top-level
  declaration (with #64's declaration index, in the file lox named) and its `blocks`: the managed block or device
  the lockfile pins by that name, or the object an extern of that name pins. Not the blocks the declaration only
  refers to: `x = And(In1: m.Q)` unused is about `x`, not `m`.
- `GET /api/check`'s `lint`: `{ available: true, findings: [{ kind, name, message, file, line, blocks }] }`.
  `name` is what lox names: a slug, a constant, a template, a block type (`unused-default`) or a user
  (`no-access-right`).
- **Never a blocker.** Apply's first step asks `lox logic check` only; findings do not refuse or warn on Apply.
  A finding meant to stay is silenced in the module, as lox says: `# lint: allow <kind>` above the declaration.

The browser, on the newest config only (as errors): an amber warning badge at the upper right of each block a
finding names, its tooltip the findings; the findings in Properties' *Lox Logic* section; on an NFC Code Touch,
the `no-access-right` findings (lox names the user, not a Code Touch); and *Lint: N findings* below Apply, folded,
by kind, each with `file:line`, the name, lox's message and a button per block that shows it on its page. The
list, too, shows only while the newest config is shown. When lint didn't run, that box says why.

## Tests (#67)

A module can hold `test "…" … end` blocks that set inputs, advance time and `expect` outputs. `lox logic test`
runs them in lox's simulator against the module compiled in memory (the lockfile is not written). vislox runs
them, shows what they say, and does not deploy while one fails.

- **Same check, same key, beside lint.** The check worker runs `lox logic test .` in the project as the second
  step of a check without errors, side by side with lint and the same way: on the same key (module files, lock,
  base, out, deployed and pending config), never from a request, without the compile lock, stopped by any action
  that starts (an edit, Compile, Apply never waits for it), and kept only when the files are still the ones it
  read. Until it is done, `test` is `{ available: true, running: true, tests }` with the last run's tests. While
  the module does not compile, the tests do not run (`test: null`).
- **A lox without test** (exit 2, `unknown command`) is found out once per lox binary, as for lint:
  `{ available: false, unsupported: true, reason }`, one quiet line in the browser, nothing refused.
- **What lox prints.** lox has no JSON for test (yet), so `check::parse_test` reads its text: a line per test
  (`test "name" ... ok`, `FAILED` or `ERROR`), the failed `expect`s under it (`    step 1: expect both.Q == 1 —
  actual 0`; with `--lox`, ` (key)` after the value) or the error, and the count it ends with (`3 tests: 2
  passed, 1 failed`, read loosely: `1 test: …`, more counts, a time after it), or `no tests in pages`. A test
  whose line says FAILED or ERROR failed, whatever the count says. Without a count, or with one that does not
  match the lines read (more tests, or more failed), lox failed or said something else (it says why on standard
  error, after `Error: `), and `test` is `{ available: false, reason }`.
- **Pinned like errors and findings.** `check::locate_tests` gives each test the line of its `test "…"` (the
  name compared unescaped, as lox prints it; its body ends at the `end` indented as that line, so a nested
  block does not cut it short), each failed `expect` its line in that test by position (among the `expect`s
  after the step lox names, the first not taken that reads the same tokens, so `1.0` and `1` or spacing do not
  matter, else the first on the same signal; comments cut as lox reads them), and both the blocks
  their `expect`s read: the slug before the port, resolved like a lint finding's name (the lockfile's managed
  blocks, `vx_<uuid>` externs, an extern's `uuid: "…"`). A test that could not run names the blocks the
  simulator has no model for (``StateV "Status" (<uuid>) is not simulated: no model for `StateV` blocks``):
  `unmodeled`, by lox's UUID, type and title, so a pass is not read as covering them.
- `GET /api/check`'s `test`: `{ available: true, passed, failed, tests: [{ name, status, file, line, blocks,
  failures: [{ step, expect, signal, cmp, expected, actual, line, blocks }], error, unmodeled: [{ uuid, type,
  title }] }] }`. `status` is `pass`, `fail` or `error` (lox counts an error as a failure, and so does vislox).
- **Apply's second step is *Tests pass*.** After *Module compiles*, Apply takes the check worker's run when it
  is of the files as they are now (the key is current, the files were looked at again, and the run is done);
  else it runs `lox logic test .` itself, shown as it runs, and keeps the result for the browsers when the check
  is of these files. So the tests are always of the config `lox config push` compiles. A test that fails or could
  not run stops Apply there, before anything is sent: 409, with the first five tests, their `file:line` and the
  `expect`s that did not hold. Apply fails closed: only a module without tests, or a lox without `logic test`,
  skips the step. Tests that could not run at all (a timeout, a lox that did not start, output vislox cannot read)
  stop it too, with *Tests couldn't run* and lox's reason; a check run like that is never taken, so Apply runs
  the tests again itself. There is no *Deploy anyway*: fix the test, or the module.

The browser, on the newest config only (as errors and lint): *Tests: N of M passed* above Apply, open while one
fails, each test with its `file:line`, its failed `expect`s (step, statement, expected and actual, at their own
line), the blocks it checks as buttons, and for a test that could not run lox's error and the blocks not
simulated. A block a failing `expect` reads (or a block the simulator lacks) gets a red ✕ badge at its upper right,
its tooltip the test, and the test in Properties' *Lox Logic* section. Apply is off while a test of the current
run fails (its tooltip says how many); while the tests run again it is on, since the server runs them itself.

Later: the same simulator could drive an offline Liveview (simulated values on the wires without a Miniserver,
#21). `Actions::run_logic(Second::Test)` and the parsed `tests` are where a separate issue would start.

## Rights and schedules a change claims (#68)

A Lox Logic module can claim more than blocks and wires: `haustuer.access = bewohner` owns who may open an NFC
Code Touch (and when), `frost.recipients = familie` who gets a push notification, and `uhr.schedule = abend` a
Daytimer's switching times. None of these is on a logic page, and whoever a set leaves out loses the right on the
next push, so the review shows them on their own. vislox reviews them, and edits switching times (#73, below);
access and recipients are #76 and #77.

- **From the compiled config, not the source.** `rights.rs` reads, with lox's own `AccessModel`, the `<PRM>`
  lists of every user and group: a `TIM=` on an `NfcCodeTouch` is an access right under that access schedule,
  an `ACT=` on a `Notification` makes it a recipient. Per block: who holds the right directly (users, then
  groups, a group with its direct members; an access right is a principal *and* a schedule, so one with two
  `<PRM>`s on a door, under two schedules, holds two rights) and who holds it in effect: directly, through a group (nested groups
  too), or, for a notification, as a member of *Full access*, who always get it. Comparing two configs gives, per
  block, the rights added and removed, and the users who gain or lose the right in effect. Rights compare by
  the schedule's UUID (two schedules may share a title); a principal with one right before and one after,
  under another schedule, is *rescheduled*. A Code Touch or notification the change deletes is there too
  (`removed_block`): everyone who held its right loses it (*Haustür removed: alice, bob lose access*). So a change made in Loxone Config shows the same way as one a claim makes, and removing
  someone from a set while their group still opens the door is not a loss.
- `view.rights` lists the blocks whose rights changed, and the Daytimers whose switching times changed (from the
  schedule comparison of #15), in document order: `{ uuid, kind, title, new_block, added, removed, rescheduled,
  lose, gain, removed_block }`, `kind` being `access`, `recipients` or `schedule`. Such a block is marked
  changed, and the change list has a line per entry (`Haustür · access: alice removed (Always)`) under its own
  filter, *rights*, coloured by what it does (`how`: added green, removed red, changed amber).
  `objects[…].rights` is each Code Touch's and notification's right, and `rights_old` the compared revision's,
  when it differs.
- **The claims.** `GET /api/edits` sends `set_claims`: each `<slug>.access|recipients|schedule = <set>` at the top
  level of the module's files (not in a template body), with its `file:line`, the line of its set's one
  declaration (`set_file`, `set_line`), and `blocks`: the block the slug names, resolved like a lint finding's
  name (the lockfile's managed blocks, `vx_<uuid>` names, an extern's `uuid: "…"`), or else the object the
  lockfile's `externals` resolved an extern by title to. Like `sources`, it is today's module, shown on the
  newest config only.
- **Apply's confirmation** asks `GET /api/apply/changes?rev=<pending>&base=<deployed>`, which loads both
  configs once and returns the devices the change reconfigures and this comparison (`{ devices, rights }`,
  without the view), within the same 6 s as the devices. It warns first: *⚠ alice loses access to Haustür (pages/access.loxl:12)*, *⚠ alice stops
  getting Frostwarnung*, then who gains a right. A schedule the Miniserver runs differently (live values, as
  before) names the claim that replaces it: *Uhr: Feiertag (claimed by pages/access.loxl:14)*. The Miniserver
  does not say when its switching times were changed in the app, so the warning has no date. When the request
  fails or does not answer in time, the confirmation says *⚠ Couldn't check access changes: …* rather than
  nothing. Neither refuses Apply.
- **Who may see it.** Names of users and groups are served with the rest of the review (the view and its
  endpoints) and nowhere else: not in a URL (the address names a block by its UUID), not in the page title,
  and never in vislox's log. Credentials (tags, codes, passwords, even hashed) never leave `rights.rs`, and no
  other attribute of a user does either.

The browser: *Rights and schedules* above the change list, while a comparison is shown, open by default and
counting the users who lose a right. Per block: its name (a button that shows it), the kind and `λ file:line` of
each claim on it; for a right, who loses it (red), the users and groups removed (red, struck through), added
(green) and given another schedule, a group's members on hover, and who gains it; for a Daytimer, each changed
mode's week before next to the week after, as bars. Properties of an NFC Code Touch has *Access*, and of a push
notification *Recipients*: the claim (*Managed by `pages/access.loxl:12` (access bewohner, pages/access.loxl:4).
Whoever the set leaves out loses access on the next push.*), who holds the right, marked against the compared
revision, *Opens for* or *Sent to* (in effect), and who loses or gains it here. A Daytimer's *Schedule* section
names its claim the same way.

Hooks for the editing issues: #73 (switching times, built) adds the `schedule` op; #76 (access) and #77
(recipients) add ops to `POST /api/edit` that write their claims into `<module>/vislox/` the same way (a set and
claims in `edits.loxl`, kept like `sched_edit::Scheds`); the review, `set_claims` and Apply's warning need nothing
more, since they read the compiled config and the module's files.

## Switching times (#73)

A DayTimer's switching times are a named `schedule` set and the timer's claim. vislox writes both into
`edits.loxl`, after the externs:

```text
extern vx_10000020_0000_0200_ffff504f94000001 = DayTimer(uuid: "10000020-0000-0200-ffff504f94000001")

# Switching times edited in vislox. A claim replaces the timer's whole schedule;
# deleting it leaves the timer as the deployed config has it.
schedule vx_uhr_schedule
	weekdays 06:30-09:00 = 21, 18:00-21:00 = 22.5 activate
	weekend off
	"Party" 18:00-23:00 = 22.5
end

vx_10000020_0000_0200_ffff504f94000001.schedule = vx_uhr_schedule
```

- **The op** is `{ "op": "schedule", "uuid": <timer>, "value": … }`. `value.modes` gives each column by its
  operating mode's UUID (the `Modes=` UUIDs Properties shows; the built-in modes have fixed ones), each a list of
  `{ from, to, value?, activate? }` in minutes of the day. `value.set` names the set: vislox's set of that name is
  rewritten (for every timer that claims it) and claimed, or with no `modes` claimed as it is (*Use the same times
  as …*). Without `set`, the times go into the set the timer claims when no other timer claims it, else into a new
  one (`vx_<name>_schedule`, a name the module, the lockfile and vislox's files do not have). `value: null` (or
  neither `set` nor `modes`) drops the claim; a set no claim names goes, and so does an extern nothing else needs.
  `value.restore` (set → columns) writes vislox's sets back first: what `previous` carries.
- **Checked before lox sees it**, with lox's rules: a DayTimer only (lox writes switching times onto nothing
  else); whole minutes up to `24:00`, the start before the end (an interval across midnight is two), no overlap in
  a column; a value (a decimal number) on every interval of an analog timer, none on a digital one (`1` is taken as
  on); every timer sharing the set the same kind; at most 48 intervals a column. One schedule per timer: a timer
  whose schedule the rest of the module claims is refused, naming that `file:line` (`set_claims`), and so is a
  block the rest of the module manages. A Daytimer added or adopted here is claimed by its slug.
- **Claimed twice.** When the module's own file claims a timer vislox claims too, lox compiles neither. Dropping
  vislox's claim is always let through, and a request of only such drops is the one edit that compiles the
  module's uncompiled changes along (they are what the drop is for). Properties says *Claimed twice* and offers
  *Remove vislox's claim*.
- **Written as lox takes it.** Every column is spelled the one way lox accepts: a built-in mode by its day word, a
  mode the module declares or externs by its slug (the lockfile's), any other by its quoted title (which must be
  unique). All seven weekdays are written, `off` where the timer is off (the claim leaves a weekday it does not
  name off all day anyway); another mode gets a column only when the op names it. Columns with the same intervals
  share a line, in weekday order, then holiday, vacation, day off, heating, cooling and the config's own modes;
  seven weekdays alike are `daily`, Monday to Friday `weekdays`, Saturday and Sunday `weekend`.
- **Checked after the compile.** The pending config must have, on each timer the request changed and every timer
  sharing its set, exactly the set's switching times (columns, intervals, values on an analog timer, activation);
  otherwise the edit is refused and the files, lock and out are put back.
- **Read back.** vislox reads the sets and claims of `edits.loxl` with lox's parser; a set or claim not named
  `vx_…`, a claim without its set, or one on a slug that is neither an extern of the file nor a block of vislox's
  files is refused, as for any statement vislox did not write. Removing a block added here, adopting a block, and
  handing one back are refused while vislox claims its switching times.
- **Not deployed and Revert.** `GET /api/edits` sends `schedules`: per timer vislox's files claim now or claimed in
  the deployed revision, `set`, its switching times (`modes`), the other timers on the set (`shared`), what the
  deployed revision claimed (`deployed`: the value a Revert sends) and `pending` (the pending config switches it
  otherwise than the deployed one; with something pending and no deployed revision, it does, as for a value); and
  `schedule_sets`, each set with its timers. Revert changes only its timer: when other timers claim the deployed
  set now, the timer gets the deployed switching times in a set of its own instead of a rewrite of theirs. Dropping a claim after Apply
  leaves the timer as deployed (lox keeps no lock state for it); before Apply, as Loxone Config had it.
- **What the app changed.** The Miniserver sends a Daytimer's schedule with the live values (#15). While they come
  in and differ from the config shown, the editor names the modes that differ: an edit made in the app is replaced
  on the next Apply, as Apply's confirmation also says (#68).

The browser: *Switching times* in Properties of a DayTimer, on the newest config, with *Edit switching times*:
a day bar per column (the seven weekdays, then the timer's other modes; *Add a column for* the other built-in modes
and the config's own, ✕ to take one out). A click on an empty bar adds an hour and a drag sets its end; dragging an
interval moves it and an end resizes it, snapped to 15 minutes (Alt: one), never past the day or into a neighbour.
The selected column's intervals below take exact times, a value, *activate* and *Delete*; *Copy to* the weekdays,
the weekend or every day; *Start from the times of* another timer of the same kind. Times are checked in the
browser first (nothing is sent while one is marked). When the timer shares its set, *Change … too* (on) rewrites
the set, off gives it a set of its own. *Use the same times as …* claims another timer's set (of a timer of the
same kind: analog or digital). The Deploy pane counts
the timers whose switching times are edited and not deployed.

## Shared sets (#73, #74)

Switching times and moods, and the claims #76 (access), #77 (recipients) and #78 (tiles) will add, share one
shape: a named set, claims on blocks, several blocks per set, one set per block. `set_claim.rs` does what is the
same for every kind, parameterised by a `SetKind` (its keyword, block type, content as an op carries it, how
content becomes lines and how lines are checked for each block): resolve the block, choose the set (named, the
block's own when no other block claims it, else `vx_<name>_<stem>`), write it checked for every claimant (refusing
one the config lost), claim it, prune, and build `previous` as a whole inverse (`restore` for every other set the
op rewrote that others still claim). It also builds the section of `GET /api/edits` (`edits_section`: per block
`set`, its content, `deployed`, `shared`, `pending`; per set its blocks, content and `stale` claimants). The browser's
Revert sends the deployed set only while no other block claims it, else the deployed content in a set of its own.
A central block's members (#75) are no such set: each claim holds its own list, and a block is in as many lists
as name it. `member_edit.rs` takes only `set_claim.rs`'s naming of vislox's blocks (`member_of`, `target_of`).

## Lighting moods (#74)

A LightController2's moods are a named `moods` set and the controller's claim. vislox writes both into
`edits.loxl`, after the externs (and after any `schedule` sets):

```text
extern vx_10000040_0000_0400_ffff504f94000001 = LightController2(uuid: "10000040-0000-0400-ffff504f94000001")

# Lighting moods edited in vislox. A claim makes the set the controller's whole mood
# list (its off mood aside); deleting it leaves the moods as last written.
moods vx_wohnzimmer_moods
	"Viel Licht": AQ1 on, AQ2 100%, AQ3 temp(100%, 2700K), AQ4 on
	"Lesen": AQ1 on, AQ2 40%
	"Abend": AQ3 temp(30%, 2700K)
end

vx_10000040_0000_0400_ffff504f94000001.moods = vx_wohnzimmer_moods
```

- **Shown from the compiled config.** `objects[…].moods` (`model.rs`, with lox's `moods` codec) has, per
  controller, its circuits (`n`, `kind`, `name`, `used`) and its moods in order (`name`, `sid`, `off`,
  `required` and `role`, each circuit's value and the line's `text`), and `selected`: the scene ids its T5 inputs
  (`Sel1`…`Sel8`) are wired to select. A circuit's kind is lox's (`Moods::prefixes`: the top four bits of its
  values, one kind per circuit): `switch` (0: on, off, a percentage), `dimmer` (1: off, a percentage), `rgb` (4),
  and `colour` or `white` (5, 6: RGB or tunable white; `white` once a mood holds a `temp(…)`); any other is `raw`,
  kept as stored. A circuit whose moods hold two kinds is `mixed`, and `error` says why: lox refuses to write
  such moods, so they are not edited here (fix them in Loxone Config). Which moods a set must keep (`required`,
  `role`: `builtin` or `motion`) is `mood_edit::required`, the one place that rule lives; the browser only shows
  it. `moods_old` is
  the compared revision's, when the moods differ, and the Changes list has a line per mood in the language's
  terms (`Wohnzimmer · mood "Lesen": AQ1 on, AQ2 80% → AQ1 on, AQ2 40%`, `… added: …`, `… removed (was …)`) and
  one for a new order of the moods both have (the off mood among them). A changed mood list marks its
  controller changed. Like lox's `loxl diff`, so a mood edited in Loxone Config or the app reads the same way.
- **The op** is `{ "op": "moods", "uuid": <controller>, "value": … }`. `value.moods` is the mood list in order,
  each `{ name, values }`: its values as a `moods` line writes them (`AQ1 on, AQ2 40%`; empty is `all off`), read
  with lox's own parser. `value.set` names the set, as for switching times: vislox's set of that name is rewritten
  (for every controller that claims it) and claimed, or with no `moods` claimed as it is (*Use the same moods
  as …*). Without `set`, the moods go into the set the controller claims when no other controller claims it, else
  into a new one (`vx_<name>_moods`, a name the module, the lockfile and vislox's files, its schedule sets among
  them, do not have). `value: null` drops the claim; a set no claim names goes, and so does an extern nothing else
  needs. `value.restore` (set → moods) writes vislox's sets back first: what `previous` carries when the op
  rewrote a set other controllers claim, so an undo gives them their moods back too.
- **Checked before lox sees it**, with lox's rules, for every controller on the set: a LightController2 only; a
  name per mood, once, one line, never the off mood ("Aus", which every controller keeps and source never lists);
  the built-in moods (scene ids 776 and 777, "Viel Licht") and any mood the motion settings name
  (`<LSConfig OpM= ScMv=>`) stay in the set, under their names (a rename is a delete); each circuit's value of a
  kind it takes (`on` on a switch only, a percentage on a switch or dimmer, `rgb(…)` on a colour circuit,
  `temp(…)` on a tunable-white one; `all <v>` sets only the circuits it fits, as lox does, so `all on` leaves a
  dimmer off); circuits `AQ1` … `AQ<outputs>`; at most 64 moods. One mood list per controller: one the rest of
  the module claims is refused, naming that `file:line` (`set_claims`, which now lists `moods` claims too), and
  so is a block the rest of the module manages. A controller added or adopted here is claimed by its slug.
- **A mood keeps its identity by name.** lox keeps a mood the controller has (its scene id, which `Sel<n>` and the
  app select it by), deletes one the set leaves out, and gives a new one the lowest free scene id: one a deleted
  mood freed, too. So a button on `Sel2` whose mood is deleted selects nothing, or the next new mood. The browser
  warns before a save that deletes or renames a mood a button selects (*Save anyway*); the server does not refuse
  it. The off mood keeps its place: first stays first, last stays last.
- **Claimed twice** (the module's own file claims the controller too) is mended as for switching times: dropping
  vislox's claim is let through and compiles the module's uncompiled change along.
- **A claimant the config lost.** When a block that claims a shared set is no longer in the pending config
  (deleted in Loxone Config), writing the set is refused, naming its UUID, rather than skipping it (lox would
  fail on the claim later). The op with that UUID and no value drops its stale claim (the one op allowed on a
  block the config does not have); `GET /api/edits` lists such claimants per set (`stale`), and the browser offers
  *Drop its claim* beside the set. The same holds for switching times.
- **Checked after the compile.** The pending config must have, on each controller the request changed and every
  controller sharing its set, exactly the set's moods in order (the off mood aside), each circuit's value as lox
  encodes it; otherwise the edit is refused and the files, lock and out are put back. lox pins the UUIDs of the
  moods it mints in the lockfile (`moods`); Discard puts the lock back with the rest.
- **Read back.** vislox reads the sets and claims of `edits.loxl` with lox's parser; a set or claim not named
  `vx_…`, a claim without its set, or one on a slug that is neither an extern of the file nor a block of vislox's
  files is refused. Removing a block added here, and handing an adopted one back, are refused while vislox claims
  its moods; adopting a block is too.
- **Not deployed and Revert.** `GET /api/edits` sends `moods` (per controller vislox's files claim now or claimed
  in the deployed revision: `set`, `moods`, `shared`, `deployed`, `pending`) and `mood_sets` (each set with its
  `controllers` and `moods`). Revert changes only its controller: when other controllers claim the deployed set
  now, it gets the deployed moods in a set of its own. Dropping a claim leaves the moods as the base has them:
  before Apply, Loxone Config's; after, the last ones written (lox keeps no lock state for the claim).
- **Circuits.** On a controller added (#59) or adopted (#72) here, `Circuit<n>: "switch"` or `"dimmer"` in its
  declaration declares circuit `AQ<n>` the way the mood editor's *Lichtkreise* tab does: the `setting` op with
  `name: "Circuit<n>"` (`AQ1` … `AQ5`, lox's `CIRCUIT_PARAMS`; `null` drops it). A circuit whose moods hold colours
  is refused: RGB and tunable-white circuits, and `Circuit6` and above, stay with Loxone Config. lox names the
  circuit after the actuator `AQ<n>` drives and refuses one that drives none; changing the kind clears that
  circuit's values in every mood, as the editor does. While vislox claims the controller's moods, the claim
  writes those values anew on every compile, so the claimed set is checked against the new kind first, for that
  controller (with no declaration: the kind the base has): `on` on a circuit about to be a dimmer is refused,
  naming the mood, before lox sees it.
- **Lint.** `lox logic lint`'s `no-actuator` finding (a percentage on a circuit without an actuator, which the
  Miniserver only switches; lox lints only claimed controllers) names the controller; the browser shows it on the
  circuit its message names.

The browser: *Moods* in Properties of a LightController2, on any revision: the circuits and moods as above. On the
newest config, *Edit moods* opens a grid, a row per mood (name, ↑ ↓, ✕; the built-in and motion moods keep their
name and have no ✕) and a column per circuit shown (*Add a circuit* for the others; a circuit not shown keeps its
values), each cell a choice of the values that circuit takes and, for a percentage, a colour or white, its
numbers. The draft is checked in the browser first. *Add mood*, *Use the same moods as …*, *Change … too* (on)
when the set is shared, *Revert*, and *Remove vislox's claim* when claimed twice, as for switching times; on a
controller added or adopted here, a switch-or-dimmer choice per circuit `AQ1` … `AQ5`. The Deploy pane counts the
controllers whose moods are edited and not deployed.

Later: a controller's motion settings, presence and other behaviour (they are not in the language yet).

## Members of a central block (#75)

A `CentralLight` drives lighting controllers, a `CentralShade` automatic blinds. Lox Logic writes the list with one
claim per central block (no named set: each claim holds its own list). vislox writes it into `edits.loxl`, after
the externs and any sets:

```text
extern vx_10000050_0000_0500_ffff504f94000001 = CentralLight(uuid: "10000050-0000-0500-ffff504f94000001")
extern vx_10000041_0000_0410_ffff504f94000001 = LightController2(uuid: "10000041-0000-0410-ffff504f94000001")
extern vx_10000040_0000_0400_ffff504f94000001 = LightController2(uuid: "10000040-0000-0400-ffff504f94000001")

# Members of central blocks edited in vislox. A claim makes the list the block's whole
# member list, in order; deleting it leaves the members as last written.
vx_10000050_0000_0500_ffff504f94000001.members = (vx_10000041_0000_0410_ffff504f94000001, vx_10000040_0000_0400_ffff504f94000001)
```

- **Shown from the compiled config.** `objects[…].members` (`model.rs`) has the attribute that holds the list
  (`rec` on a central block, `Objects` on an alarm), whether vislox edits it (`editable`), the type a member is
  (`takes`), and the members in order, each a link target (title, type, page) or `missing` when the config has no
  such object. `members_old` is the compared revision's, when the list differs, and the Changes list says it by
  name in place of the raw `rec=` line: `Alle Lichter · member added: Diele`, `… member removed: …`, and one line
  for a new order of the members both revisions list.
- **The op** is `{ "op": "members", "uuid": <central block>, "value": [<member uuid>, …] }`: the whole list, in
  the order the app lists them; `[]` empties it (lox writes no `rec=` then). `value: null` drops vislox's claim:
  the members stay as last written (lox keeps no lock state for it), so before Apply that is the base's list.
  `previous` is the list vislox claimed before (null: none).
- **Checked before lox sees it**, with lox's rules: a `CentralLight` or `CentralShade` only (an `Alarm`'s or
  `SmokeAlarm`'s `Objects=` stays with Loxone Config: it names hidden references wired to the alarm, which the list
  does not own); each member a `LightController2` under a `CentralLight`, an `AutoJalousie` under a
  `CentralShade`, listed once, never the central block itself; at most 200. A member Loxone Config drew is named by
  an extern of the file, one added or adopted here by its slug; one the rest of the module manages is refused
  (vislox's files name only their own blocks and Loxone Config's, and lox refuses an extern of a managed block):
  list the members in the module's own file instead. One claim per central block: one the rest of the module
  claims is refused, naming that `file:line` (`managed::members_claims`, sent as `members_claims`), and so is a
  central block the rest of the module manages; a central block added or adopted here is claimed by its slug.
- **Claimed twice** (the module's own file claims the members too, which lox refuses) is mended as for switching
  times: dropping vislox's claim is let through and compiles the module's uncompiled change along. Properties says
  *Claimed twice* and offers *Remove vislox's claim*.
- **A member the config lost** (deleted in Loxone Config) leaves an extern lox cannot resolve. `GET /api/edits`
  names it (`stale`), and the browser offers *Drop the missing members*: the list without them. A central block
  the pending config lost can only have its claim dropped (the op with its UUID and no value).
- **Checked after the compile.** The pending config must list, on each central block the request gave a list,
  exactly those members in that order; otherwise the edit is refused and the files, lock and out are put back.
- **Read back.** vislox reads the claims of `edits.loxl` with lox's parser; a claim not on a `vx_…` slug, or naming
  a slug that is neither an extern of the file nor a block of vislox's files, is refused. Removing a block added
  here, handing an adopted one back, and adopting a block are refused while a member list names it (as the central
  block or a member).
- **Adopting a central block** (#72): `lox logic adopt` lifts its members as a claim. vislox leaves that claim
  out, so the block keeps the list Loxone Config wrote (a dropped claim leaves the members as last written) and
  adopting changes nothing; editing the members later claims them by the block's slug.
- **Not deployed and Revert.** `GET /api/edits` sends `members`: per central block vislox's file claims the
  members of now or claimed in the deployed revision, `members` (the UUIDs now, null: no claim), `deployed` (the
  value a Revert sends), `pending` (the pending config lists other members than the deployed one), `stale` and
  `missing`.

The browser: *Members* in Properties of a central block or an alarm, on any revision, each member a link. On the
newest config, *Edit members* on a central block lists them with ↑ ↓ and ✕ and *Add member…*, which opens the
Ctrl+K palette as a picker (Ctrl+K itself while the editor is open): only the blocks of the type the central block
takes, not the central block, not one listed already, not one the module manages. *Save*, *Cancel*, *Revert*,
and *Remove vislox's claim* when claimed twice. The Deploy pane counts the central blocks whose members are edited
and not deployed.

## Undo and redo (#62)

Undo is not a second way to change the module: it sends the edit's `undo` back to `POST /api/edit` as an edit of
its own, against the pending config shown then. So it is checked, written and compiled like any edit, refused
like any edit (a module with uncompiled changes, a block deployed since, a stale `base`), and it never deploys.
The reply's `undo` is the redo, and the redo's reply the next undo, so each step is built from what the server did,
not guessed in the browser.

- The history lives in the browser tab. Every edit of the page (values, Revert, *Add block*, *Remove*, moves,
  *Arrange*, wires), every undo and every redo goes through one function, which sends it and keeps what the reply
  says: a new edit (its ops, its `undo`, a name made from the ops) goes on the undo stack and empties the redo
  stack; an undo moves the edit to the redo stack with the reply's `undo` as its redo, and a redo the other way.
  Up to 50 edits are kept. An edit that changed nothing is not kept; an undo or redo that changed nothing (the
  config already was that way) moves its edit across with its ops as they were. A reply with no `undo` empties
  both stacks, since what came before no longer leads to this config, and says why (`undo_error`).
- *Undo* and *Redo* sit next to *Add block* (Ctrl+Z and Shift+Ctrl+Z, ⌘ on a Mac; Ctrl+Y redoes too, but not on a
  Mac); each one's tooltip names the edit it undoes or redoes. In a text field the keys are the field's own. The
  list next to them shows the recent edits, those undone struck through, and why the history was cleared.
- Like every edit, undo and redo change the newest config while it is shown; on an older revision they are off.
  Each step is sent against the config the stack was built on, after the edit or step before it was compiled and
  the History shows it: a Ctrl+Z right after a save waits for the save and undoes it, and quick presses run one
  after another (up to three wait), never against a config the server has moved past. While they run, the other
  edits are off.
- **A block added again gets new UUIDs**: lox mints them. The reply names the old UUID next to the new one
  (`added[].restores`), and the browser renames it in every edit it keeps, so an undo of a value edited before a
  *Remove* changes the block that came back.
- **The history belongs to one pending config**: the blob it was made on, the server's last Apply, Sync or Discard,
  and the deployed revision. When any of them changes and this tab did not do it (an edit in another tab, `lox
  logic compile` run elsewhere, Discard, Apply, Sync, a Pull), the ops kept may no longer undo anything sensible,
  so the history is cleared and the list says why ("Discard dropped the pending config", "the pending config
  changed outside this tab"). It is not rebased: an edit someone else made is theirs to undo. A moment after this
  tab's own edit, before the History has the new compile, does not count.
- **One tab, kept across a reload**: the history is this tab's, stored in `sessionStorage` under the pending
  config's blob (`vislox.undo.<blob>`) with the last outcome and deployed revision it was made on. A reload of the
  tab picks it up when the config is still exactly that one. Another tab has its own history, and an edit made
  there clears this one. After a server restart the last outcome is unknown; the blob and the deployed revision
  decide.
- A refused undo or redo leaves the history as it was and says why.

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
| #61 wire | `x.Port <- y.Q` in `edits.loxl` | Adds a wire between objects Loxone Config drew and blocks added here, and removes one vislox drew. A wire Loxone Config drew cannot be removed. |
| #72 adopt | `lox logic adopt` on a copy; its declaration in `<module>/vislox/<page>.loxl` | The block keeps its UUIDs, the config stays as it is; then edited like a block added here. |
| #73 switching times | `schedule vx_… end`, `x.schedule = vx_…` in `edits.loxl` | A DayTimer, extern or added/adopted here; several timers may share a set. |
| #74 moods | `moods vx_… end`, `x.moods = vx_…` in `edits.loxl`; `Circuit<n>:` | A LightController2, extern or added/adopted here; several controllers may share a set. Circuits only on one added/adopted here. |
| #75 members | `x.members = (a, b)` in `edits.loxl` | A CentralLight or CentralShade, extern or added/adopted here; members externs or blocks added/adopted here. An alarm's `Objects=` stays with Loxone Config. |

Blocks the module already manages (they are in the lockfile) are refused: they are edited in their own `.loxl`.
The blocks vislox added or adopted (#72) are the exception: their `.loxl` is vislox's.

## What lox cannot do, so vislox does not offer it

- Parameters stored on the block element of a block the module does not manage: a Formula's text, a Daytimer's
  On/Off labels, a memory flag's type, a virtual input's Analog, MinVal, MaxVal and DefVal, and `Desc` on logic
  blocks. lox takes them only in the declaration of a block it manages: adopt the block first (#72, built), then
  edit it as a managed block (#79).
- A room or category on an object that has none.
- Handing a value back after Apply: deleting a claim makes lox restore the value from before the claim, which
  undoes the applied change. So once applied, the module owns the value, and a later change in Loxone Config is
  overwritten on the next Apply. Revert before Apply works.
- Moving an object it does not manage: a block Loxone Config drew, an input or output reference (`InputRef`,
  `OutputRef`), a note, a comment. lox never writes their position; they are moved in Loxone Config, or adopted
  first (#72). Arranging lays out only managed blocks, and leaves one wired to a reference the module does not
  manage beside that reference.
- Removing a wire Loxone Config drew, or reconnecting it: lox takes out only the wires its source draws. Remove it in
  Loxone Config, then draw it here.
- Wiring an input Loxone Config inverted (`Inv=`): lox leaves that connector to Loxone Config.
- A wire to an object on another page is not drawn by dragging (both ends must be on the page shown); lox wires
  across pages directly, and Loxone Config shows such a wire as a link on both pages.
- New blocks of some types. vislox offers the page blocks of lox's verified builtin table (`loxl types`) that
  lox creates and that work as placed; a test compiles each one with lox. Not offered:
  - virtual inputs, virtual HTTP, UDP and text inputs, virtual outputs and their commands, Modbus servers,
    devices and their I/O, and operating modes: objects of the object tree, not of a page (#84);
  - the NFC Code Touch, Intercom and Touch Pure Flex controller blocks, which exist because Loxone Config paired
    the device (lox adopts them, never creates them);
  - blocks lox creates but that do nothing until they are set up in Loxone Config: the burglar and smoke alarms,
    the room controller, the irrigation, and the central light and shade blocks (the members of one the config
    has are edited here, #75);
  - every type outside lox's table (lox refuses it: "Refuse, never guess"), and templates (#83).
- On a new block: a program's text (`Code:` takes only `file(…)`), a wire when it is added (draw it once it is
  on the page, #61) or an expression on an input (#82),
  a position when it is added (lox places it; it can be moved after), and a room or category (lox has no claim for them on a managed block). Its settings
  (`Formula:`, `Type:`, `At:`, …) are given when it is added; changing them later is #79.
- A block on a page whose title an earlier page has, and a new page (#88).

## The rest of the language

Each of these plugs in as new op kinds on `POST /api/edit`, written into `<module>/vislox/`:

| Issue | Lox Logic |
|---|---|
| #59 (built) | new blocks (`slug = Type(…)`, `page`), `removed` for one not deployed |
| #60 (built) | positions of managed blocks (the lockfile's `layout`), `--relayout` |
| #61 (built) | wires (`<-`) |
| #62 (built) | undo and redo (the edits' own `undo`, sent back as an edit) |
| #72 (built) | adopt a block into the module (`lox logic adopt`) |
| #73 (built) | Daytimer switching times (`schedule`, `.schedule`) |
| #74 (built) | lighting moods and circuits (`moods`, `.moods`, `Circuit<n>:`) |
| #75 (built) | members of a central block (`.members`) |
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

Around the module: #64 (mark managed blocks, built), #65 (`lox logic check` errors, built), #66 (lint, built), #67 (`lox logic test`, built),
#68 (review claimed access, recipients and schedules, built), #70 (removals on Apply, built: a block the module
lost is deleted only with consent to exactly that list, see [features.md](features.md#apply-sync-and-discard)).

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
- A wire is drawn by dragging from a port to a port on another block, an output to an input either way round
  (the newest revision, with editing on). A port's target is the size of its stub; a click on it without a drag
  selects its block. While dragging, the ports it may end at are blue and those it may not
  red, with the reason on hover; dropping on one of those says why and sends nothing. The drop is compiled.
- Clicking a wire opens its card: its ends and kind, and *Delete* (also the Delete key) and the ends to drag
  elsewhere for a wire vislox drew, *Revert* for one drawn or taken out here and not deployed, or why it cannot be
  changed here (Loxone Config or the module drew it). The Deploy pane counts the wires drawn and taken out here.
- *Arrange*, next to *Add block*, lays out the page's managed blocks afresh. It is off, with the reason, where
  the server would refuse it (`arrange` in `GET /api/edits`).
- A refused move puts the block back where it was. A move that compiled stays drawn even when showing the new
  revision fails; Properties says so.
- `GET /api/edits` lists what vislox's files claim now and in the deployed revision, which of it is not
  deployed, the blocks vislox added (`blocks`: slug, UUID, type, page, file, and whether the deployed config has
  it), the other blocks the module manages (`managed`), where the module makes each managed block (`sources`,
  #64), the blocks lox can move (`movable`: managed and drawn on a page) and
  those the pending config draws elsewhere than the deployed one (`moves`: UUID, position, deployed position),
  for each page with such a block whether *Arrange* can lay it out (`arrange`: page UUID → null, or why not), and
  the wires vislox's files draw now or drew in the deployed revision (`wires`: `from`, `to`, `now`, `deployed`), the
  wires the rest of the module draws (`module_wires`) and those an adopted block brought along
  (`adopted_wires`), why vislox leaves those and Loxone Config's alone (`wire_why`), and which blocks can be
  adopted (`adopt`, #72). It
  is worked out again only when vislox's files, the lockfile, the deployed or
  the pending config changes (`actions.edit.key`); the browser fetches it only then.
