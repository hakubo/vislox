# Editing in vislox

vislox becomes an editor in steps: edit values (#58), add blocks (#59), move blocks, inputs and outputs (#60),
make and change connections (#61), undo and redo (#62), adopt the blocks Loxone Config drew (#72), edit a
Daytimer's switching times (#73), a lighting controller's moods (#74), a central block's members (#75), who may
open an NFC Code Touch (#76), push notifications: who gets them and what they say (#77), the tiles of a Touch
Pure Flex (#78), a managed block's label and settings, its program among them (#79), deleting and renaming a
managed block (#80), `let` constants and `defaults` (#81), expressions and named signals (#82), template
instances (#83), virtual inputs, virtual and Modbus devices (#84), Tree and Air hardware: statistics and new
Tree devices (#85), and operating modes and calendar entries (#86). This page is the design they share. #58 to #62,
#72 to #86 are built (#81 to #84 and #86 in part: see their sections); the rest says where each step plugs in.

## The rules

- **Only a `lox.toml` project is edited.** It is the only source with a pending revision, Apply and Discard.
  A `.Loxone` file, a backup or a `lox config init` repository stays read-only.
- **Edits go into the module, never into a `.Loxone`.** `lox config push` compiles the module again before it
  uploads, so a change made only in `out.Loxone` would be lost on Apply, and `base.Loxone` is what the
  Miniserver runs.
- **vislox writes one place: its own files under the module directory**, `<module>/vislox/`: `edits.loxl` for
  the values it claims, the wires it draws and the constants it makes (#81), and one file per page for the blocks it adds
  (`<module>/vislox/obergeschoss.loxl`), `_periphery.loxl` for the virtual inputs and devices (#84), operating modes and
  calendar entries (#86) it adds,
  `_hardware.loxl` for the Tree and Air hardware it declares (#85), and a file per program a block of them runs
  (`vx_code16_1.c`, #79).
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
           { "op": "rename", "uuid": "…", "slug": "vx_nachlauf" },
           { "op": "tidy" },
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
           { "op": "members", "uuid": "…", "value": [ "<member uuid>", "<member uuid>" ] },
           { "op": "access", "uuid": "…", "value": { "grants": [
               { "kind": "group", "uuid": "<group uuid>", "schedule": "<access schedule uuid>" } ] } },
           { "op": "recipients", "uuid": "…", "value": { "recipients": [
               { "kind": "user", "uuid": "<user uuid>" }, { "kind": "group", "uuid": "<group uuid>" } ] } },
           { "op": "add_block", "page": "<page uuid>", "type": "OutputRefLM",
             "settings": { "mirrors": "<notification uuid>", "On": "Frost", "MinimumTime": "12h" } },
           { "op": "let", "name": "vx_nachlauf", "value": "30min" },
           { "op": "param", "uuid": "…", "port": "Time", "value": "vx_nachlauf" },
           { "op": "expr", "uuid": "…", "port": "I2", "value": "vx_….Q and vx_….AQ >= 28" },
           { "op": "signal", "slug": "vx_warm", "page": "<page uuid>", "value": "vx_….AQ >= 28",
             "title": "Warm" },
           { "op": "instance", "slug": "vx_ost", "page": "<page uuid>",
             "value": { "template": "Treppenlicht", "args": { "taster": "vx_…", "dauer": "5min" } } },
           { "op": "add_block", "page": "<folder or device uuid>", "type": "VirtualHttpIn", "title": "Zähler",
             "settings": { "Address": "http://192.0.2.10/status", "Poll": "1min" } },
           { "op": "statistics", "uuid": "<Tree or Air I/O uuid>", "value": "on" },
           { "op": "add_block", "page": "<Tree extension uuid>", "type": "TreeDevice", "title": "Melder Flur",
             "settings": { "kind": "PresenceSensorTree", "branch": "left" } } ] }
```

`adopt`, `adopt_page` and `unadopt` go in a request of their own (#72), and so does `rename` (#80). `restore_block`
is what undoes a `remove_block` of a deployed block (#80); vislox makes it, see *Deleting and renaming*.

- `base` is the git blob id of what the browser showed: the newest compile, or the deployed config when nothing
  is pending. When the file on disk is another one (someone compiled, applied or discarded), the edit is refused
  with 409, and the browser shows the new revision first.
- `value: null` drops vislox's claim: lox puts back the value Loxone Config had before.
- The reply has, per op, what undoes it (`previous`), the new pending blob id, and the blocks it added with the
  UUIDs lox minted (`added: [{ slug, uuid }]`). For a value, `previous` is the value vislox claimed before: the
  op with that value undoes it. For `remove_block` it is the `add_block` that adds the block again (a deployed or
  adopted block: the `restore_block` that brings it back with its UUIDs, and for `restore_block` the `remove_block`;
  for `rename` the `rename` back; for `tidy` a `tidy`, #80); for
  `add_block` it is null, and `remove_block` with the new UUID undoes it. For `move` it is the `move` back to
  where the block was; for `relayout`, the `place` that puts each block back (one op however many
  blocks the page has; `place` moves several managed blocks as `move` does), and for `place`, the `place` back. For `wire` it is the
  `unwire` of the same ends, and for `unwire` the `wire`. For `adopt` and `adopt_page` it is the `unadopt` of
  the blocks adopted, and for `unadopt` the `adopt` with the same slugs: a block keeps its UUIDs both ways. For
  `schedule` it is the `schedule` of what the timer claimed before: its set and that set's switching times, or
  no claim; when the op rewrote a set other timers still claim, it also carries that set's old switching times
  (`restore`), so the undo gives them back too. `moods` is the same for a controller's moods (#74). For
  `members` it is the list vislox claimed before, or null (#75). `access` is as `moods`, for a Code Touch's
  rights (#76), `recipients` for a notification's recipients (#77), and `tiles` for a Touch Pure Flex's tiles (#78).
  `let` is a value op (#81): its `previous` is the constant's value before, or null when the op made it. `expr`
  is a value op too (#82): what vislox's files fed the input before, or null; `signal`'s `previous` is the `signal`
  op that puts back what was (null `value`: the op made the signal). `instance` is as `signal`, for a template
  instance (#83). `statistics` is a value op (#85): what vislox's file claimed before, or null. `entry_mode` and
  `entry_date` are value ops (#86): the mode's UUID and the date as written before, or null. A Tree device created
  here is added and removed as a block: `remove_block`'s `previous` is the `add_block` that creates it again (a new
  UUID) or, once deployed, the `restore_block` that brings it back with its UUID.
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
  lost). vislox drops that line once the lockfile no longer has the block. A deployed block is deleted, and any
  block of vislox's files renamed, as *Deleting and renaming managed blocks (#80)* says.
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
  vislox does not take them out yet. It is deleted and renamed as a deployed block added here is (#80).
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

Later: taking out a wire an adopted declaration brought along is left to a later step. Its label and settings are
edited as for a block added here (#79, see *Block settings*); it is deleted and renamed as #80 says.

## Deleting and renaming managed blocks (#80)

A block of vislox's files, added here (#59) or adopted (#72), is deleted with `removed <slug>` and renamed with
`moved <old> -> <new>`: without them lox refuses a lockfile entry the source lost, or mints a renamed block anew
and every wire Loxone Config drew to it breaks. The blocks the module's own files declare are deleted and renamed
there (`lox logic rename`); vislox refuses them, naming the file.

```text
# Deleted here: lox takes them out of the config.
removed vx_monoflop_1

# Renamed here: the blocks keep their UUIDs.
moved vx_nach -> vx_nachlauf
```

- **Delete** (`remove_block` of a deployed or adopted block). Its declaration goes, with the wires out of it its file
  wrote (an adopted block's `<-` lines) and the wires drawn here at it (#61), and `removed <slug>` is added to its
  file. lox deletes the block from the pending config and moves its lockfile entry to a removal tombstone, which
  keeps deleting it from the base until a base without it is compiled. The review shows the block removed (red, a
  dashed ghost). Apply deletes it on the Miniserver, and asks first like any removal (#70: the tombstone is in the
  project's lockfile, so *Review the removals* lists it). After the compile, vislox checks that the config no longer
  has it. A block not deployed yet is taken out as before (#59).
- **Refused** while vislox's file claims a set on it or names it in one (Revert that first), and while another block
  of vislox's files takes a wire from it in its declaration (an adopted block bound to it: delete that one first, or
  hand it back), and while a wire drawn here at it names an end vislox cannot find by UUID (its undo could not draw
  it again). `blocks[].delete` in `GET /api/edits` says why (`edit::delete_refusal`: the checks the op makes first,
  which change nothing), and *Delete* is off with that reason. Deployed means the deployed revision has its UUID,
  case aside, for the browser and the server alike. A wire into it from the module's own files is lox's to refuse, and its words come back.
- **Undo** is `restore_block`: what `remove_block`'s `previous` names, never written by hand. It carries the
  declaration and its wire lines as vislox wrote them (read back with vislox's own parser: one block, its slug the
  tombstone's), its program, the wires drawn here (by UUID), vislox's externs its declaration names (UUID → type),
  its lockfile entry (`objects.<slug>`: UUIDs, ports, layout, page) and its `extern_wires`. It is accepted only while
  the lockfile keeps the block's tombstone and the base still has it, and the entry is checked, never trusted: only
  the fields lox writes, the tombstone's UUID, the declaration's type, each port one of the connectors the base's
  object has (by key and UUID), a rectangle for a layout, and the page it is drawn on; a tampered or stale entry is
  refused and changes nothing. A wire drawn here whose other end is gone is left out and said (`warnings` in the
  reply, shown with the undo); the block comes back all the same. Then the entry goes back, the tombstone goes, the
  entry's `extern_wires` come back (and leave `removed_wires`), and the `removed` line goes, so lox writes the block
  with its old UUIDs and the config is as it was. After the compile vislox checks that the config has it again under
  its UUID. Once the deletion was applied (or discarded) there is no tombstone, and it is refused.
- **Rename** (`rename`, a request of its own): the slug is `vx_` and lowercase letters, digits and `_`, not a
  reserved word, without `__` (lox's for expression blocks), not `vx_<uuid>` (vislox's name for an object of
  Loxone Config's), and not a name of the module, its lockfile (tombstones too) or vislox's files. lox's own rename
  (`rename_slug`, the engine of `lox logic rename`) runs over vislox's files, so the declaration, wires, bindings,
  set claims and tile targets follow, and `moved <old> -> <new>` is added; a block's program file follows its slug
  (`vx_nachlauf.c`). A rename of a rename lox has not applied yet is one rename, and one back to the old slug none.
  lox moves the lockfile entry, so the block keeps its UUIDs, layout and wires.
- **The config does not change.** A block without a name shows its slug, so it keeps the old slug as its name
  (`vx_nachlauf = Monoflop("vx_nach")`); renamed back to a slug it shows by that name, the name goes again. After the
  compile vislox compares the pending config with the one before (`loxone_config::diff`) and refuses a rename that
  changed anything. Nothing is pending afterwards unless something else is.
- **The lines afterwards.** A `removed` line is done once lox keeps the tombstone (the lockfile no longer has the
  block), a `moved` line once lox moved the entry; `lox logic lint` reports them as `stale-lifecycle`. Every edit
  drops them first (`Files::tidy`), so they never pile up. `GET /api/edits`' `stale_lines` lists them as written, the
  Deploy pane offers *Drop them*, and `tidy` (a request that asks for nothing else) drops them and compiles; the
  config does not change.

## Block settings (#79)

Loxone Config keeps some of a block's settings on the block element rather than on a connector: a Formula's text,
a PulseAt's time and pulse, a memory flag's type, a Daytimer's On and Off labels, the name the app shows (`Desc`),
a meter's units, an automatic blind's *use sunshine*, a program block's program. lox takes them only in the
declaration of a block the module manages (`lox logic adopt` lifts them; a claim on an extern is refused), so they
are edited on a block of vislox's files: one added here (#59) or adopted (#72).

```text
vx_abends = PulseAt("Abends", Desc: "Licht abends", At: 06:58, Pulse: "time")
vx_merker = Memory("Anrufer", Type: "text")
vx_hue = Code16("Hue", Code: file("vx_hue.c"))
vx_nachlauf = Monoflop("Nachlauf", Time: 30min)
```

- **The ops** are the ones values use: `title` for the label, `setting` (by the Lox Logic name, `At`, not `Sec`)
  for a setting, `param` for a constant. `previous` is the value the declaration gave before (none: the binding
  goes, and the block is as the base has it), so undo and Revert work as for any value. The settings each type
  takes, with how each is written, are lox's table (`connectors::attr_params`, what `loxl types` lists), checked
  before lox sees them: a time of day (`At: 06:58`), one of a few words (`Pulse:`, `Type:`), `on`/`off` (`Sun:`),
  text (`Desc:`, `Formula:`, `On:`, `Off:`, the meter units; empty only for `On:`/`Off:`), a number (`MaxLvl:`).
  An empty field drops the binding: Loxone Config's value again.
- **The label** follows the rules a save applies: one line, no tab, not empty, and no character a Loxone Config
  save would rewrite (`{`, `}`; `%`, `<`, `|` and spaces at either end become `_` on most types): the error says
  what the save would write.
- **A program** (`Code:` on a `Code16`) is a file of its own next to the block's file, `<module>/vislox/<slug>.c`,
  named by `Code: file("<slug>.c")` (lox reads `file(…)` relative to the `.loxl` file that names it). The op's
  value is the program's text, so `previous` and undo carry it; CRLF becomes LF, as lox reads the file. Edited, the
  block keeps its file; dropped, the binding and the file go, and the block runs the program the base has
  (Loxone Config's). A program file no block names, or a block whose program file is gone, is refused when vislox
  reads its files, naming the file. An adopted program block runs Loxone Config's program until its program is
  edited here: `lox logic adopt` never writes `Code:`.
- **Units.** A constant is shown and edited as Lox Logic writes it: a port documented in milliseconds, seconds or
  minutes with the largest of `h`, `min`, `s`, `ms` that gives a whole number and the same digits back (`1800` on
  a seconds port is `30min`, `0.5` is `500ms`; zero and the other ports stay bare). A typed value may carry a unit
  (`30min`, `250ms`, `70%`): a time on a time port (exactly, in the port's unit: `10s` on a minutes port is not),
  `%`, `K` or `°` on a port in that unit, any on an undocumented port. It is written as typed (`Time: 30min`,
  `x.Time = 1h` on a claim) and lox scales it. The same goes for a constant of a block Loxone Config drew (a claim)
  and of a new block.
- **Read-only elsewhere.** Properties has a *Settings* section on every block whose type has such settings, by
  their Lox Logic names and values (`At 18:58`, `Pulse sunset`, a program folded to its line count), marked against
  the compared revision (a program as a line diff); the raw attributes they are stored in leave *Advanced*. On a
  block Loxone Config drew it says *Read-only: … Manage with Lox Logic to edit* (the adopt button of #72); on one
  the module's own files manage, the file and line to change it in.
- **The Changes list** reads them the same way: `Abends · At: 18:58 → 06:58`, `Abends · Pulse: time → sunset`,
  `Hue · program: - … · + …` (the changed lines), `Nachlauf · Time: 30min → 1h`.
- Not offered: the settings lox refuses by name (an alarm's members, a meter's `Rec=`, `StatsType=`, a room
  controller's, `Task=`; see lox's `connectors::unclaimed_reason`), and any setting on a block the module does not
  manage.

## Constants and defaults (#81)

`let` constants and `defaults` statements set values in many places at once, and the compiled config keeps no
trace of either: lox writes the value into each port's `Def=`. vislox reads them back from the module's source.

```text
let temp_schwelle = 28                # the module's own file
teiler.In2 = temp_schwelle
defaults Monoflop(Time: nachlauf)

let vx_nachlauf = 30min               # vislox's edits.loxl
vx_60000002_0000_0000_ffff504f94000001.Time = vx_nachlauf
```

- **Where a value comes from.** `GET /api/edits` sends `constants`: `lets` (name, value as written, `file`, `line`,
  `uses`: the ports bound to it, through a default too, and `ours`: declared in vislox's files), `defaults` (`type`,
  `file`, `line`, `params`, `uses`) and `uses`: object UUID (lowercase) → port key → `{ kind, name, constant, value,
  file, line, at_file, at_line, ours, at_ours }`. `kind` is `let` or `defaults`; `name` the constant or the type;
  `constant` the constant a default names (`defaults Monoflop(Time: nachlauf)`); `file`/`line` the statement;
  `at_file`/`at_line` where the block binds it (the argument, or the `x.Port = name` line) or, for a default, the
  block's declaration; `at_ours` whether that is a file of vislox's. They are read with lox's parser, at the top
  level of each file (a template's body is expanded per instance, which #83 reads): a managed block's arguments (the lockfile's `objects`), an
  extern's `x.Port = name` and its folded arguments (its `uuid: "…"`, else `externals`), and every managed block of
  a type a `defaults` statement names that does not bind the port itself (lox's `defaults_taken`). An alias (`let x
  = y.Q`) names a port, not a value, and is left out; a file that does not parse is left out (`lox logic check`
  says why, #65). Like `sources` (#64) they are the module as it is now: shown on the newest config only.
- **Properties** shows the source beside the value: `Time 30min (nachlauf)`, `ModbusPollingCycle 60 (defaults
  ModbusASensor)`, the tooltip with the statement's `file:line` and how many ports take it.
- **The Changes list** puts the parameter changes one constant or one `defaults` statement causes together, under
  a heading `let vx_nachlauf = 45min · 2 changes` with the statement's `file:line` (a default that names a constant
  goes under the constant). Next and previous go in that order. A single such change is marked `(let x)`. Each
  `param` change carries its connector's key (`port`) for this.
- **The `let` op** makes, changes or drops one of vislox's constants: `let vx_<name> = <value>` in `edits.loxl`.
  The name is `vx_` and lowercase letters, digits and `_`, not one vislox names an object by, and not taken in the
  module or its lockfile. The value is what a port takes: a number, one with its unit (`30min`, `70%`; lox scales a
  time into each port's unit), a time of day (`19:00`) or `on`/`off` (lox checks it against every port bound to
  it). A text constant is not made here. `value: null` drops it, refused while a port of vislox's files is bound to
  it. `previous` is the value before, or null when the op made it, so undo drops a constant it made. The names it
  is checked against are the module's own files' (vislox's directory left out) and vislox's files as the request
  has changed them so far, so a request may drop a constant and make it again.
- **Not deployed.** `GET /api/edits`' `claims` lists each of vislox's constants as a `let` op, `deployed` the value
  the deployed revision's file gave it, and `pending` while the two differ (a constant leaves no trace in the
  config to compare). Properties of a block that takes it shows *Constant vx_… 45min not deployed* with *Revert*,
  which sends `let` with the deployed value; one made here has none, and goes once the ports bound to it are
  reverted.
- **Binding a port** is the `param` op with the constant's name as the value (`"value": "vx_nachlauf"`): on a claim
  it is written `x.Time = vx_nachlauf`, on a block of vislox's files `Time: vx_nachlauf`. The name may be vislox's
  constant or one the module declares. Any other value binds the port to a value of its own, as before. Only a
  request with such a bare name reads the module's own files for their constants; when they do not read, it is
  refused with lox's error rather than "not a number". A bare name of vislox's constant on an adopted block (#72)
  reads as the value, never as a wire from an object (`PageFile::parse_with` knows vislox's constants). Adopting
  keeps the module's constants and aliases (`let x = y.Q`) as bare names.
- **Make a constant** (Properties, *Edit values*, under an unbound constant field) names the field's value: one
  request of `let` and `param`, undone together. Another block takes it by its name, typed into its field (the
  field offers the constants).
- **Changing a value a constant gives** asks first: *Change the constant (N uses)*, which sends `let` and changes
  every port bound to it, or *Only this block*, which sends `param` and binds this port to a value of its own. Two
  fields of one constant given different values are refused in the form, naming the other field.
- **Not offered, a product decision:** changing a `let` or a `defaults` line of the module's own files. vislox
  writes only its own files (see *The rules*), so a value from such a constant or default is changed only on the
  block: *Only this block*, on a block of vislox's files or a claim. Where the module's own file binds the port (`x.Port
  = name` on an extern), the field is read-only and names that line, and the server refuses a `param` claim on it
  (`… is bound in pages/x.loxl:3, the module's own file`): a claim of vislox's on the same port would claim it
  twice. Dropping vislox's claim is let through. The `let` op on a constant of the module's own files is refused, naming why. vislox never writes
  a `defaults` statement: it changes every managed block of the type, the module's own among them.

## Expressions and named signals (#82)

Lox Logic writes logic as expressions: `jal.AutoShade <- sonne.Q and temp.AQ >= 28` on an extern, `In1: …` in a
managed block's declaration, or a named signal `beschatten = sonne.Q and temp.AQ >= 28`. lox desugars each into
`And`, `Or`, `Not`, comparator and `Formula` blocks labelled with their sub-expressions (`<=` as `≤`, `<` as `＜`,
`%` as `％`), expression-owned in the lockfile and named `<sink>_<port key>__<op><n>`, or `<signal>__<op><n>` with
the signal's top block `<signal>` itself (hand-owned). lox re-derives them on every compile, so they are never
edited as blocks: the expression is.

- **Read.** `GET /api/edits` sends `expressions`, read with lox's parser at the top level of each file (a template's
  body's expressions are its instances' blocks, #83): `id` (the blocks' prefix; a second expression on one prefix is `<prefix>#2`), `kind` (`wire`, `argument` or `signal`), `sink` (`jal.AutoShade`, or the
  signal's slug), `to` (`[object UUID, connector key]` of the input it feeds), `text` (as lox prints it), a signal's
  `title`, `file`, `line`, `ours` (in vislox's files), `top` and `top_type` (its top block: the signal's, or the last
  one lox's post-order count names), `blocks` (the blocks lox names for this expression, by the same count shared
  by the expressions of one prefix, so each block is exactly one expression's; the top one first), `deployed` (the block that feeds the sink in the deployed config, or the signal's by UUID, when lox makes
  that type: its `label`, read back as `text`, and the chain's `blocks` then: those wired into it that are this
  expression's now or gone from the pending config), `now` (the pending config's top label read back the same way)
  and `pending` (the chain as compiled differs: each block's type, label and attributes and what feeds each
  connector, so a labelled signal whose expression changed is pending too). A signal of vislox's carries `revert`,
  the ops that put the deployed revision's files back: each input it feeds that they did not feed from it gets back
  what they fed it (`expr`), then the `signal` op; and `committed`. Like `sources`, they are the module as it is now: newest config only.
- **The canvas** draws an expression's blocks as one block over the area they take, showing what it feeds and the
  expression; its blocks and the wires between them are hidden, and other blocks in that area stay on top of it,
  taking their own clicks. A click opens it (and selects its top block): the
  blocks show inside a dashed frame titled with the expression, whose title folds it again. Selecting a block of a
  folded expression (the tree, Ctrl+K, the Changes list) opens it.
- **Properties** of any block of an expression, and of the block it feeds, has an *Expression* section: the
  expression, what it feeds (or the signal's output and label), how many blocks lox made, the file and line, and
  *Not deployed* with what the deployed config has. One of the module's own files is read-only: *change it there*.
- **The Changes list** puts every change to an expression's blocks, now or deployed, under one heading: `expression
  on jal.AutoShade: <deployed> → <now>` when the deployed revision is compared and the two top labels, read back
  alike, differ (`(new)` for an expression it does not have, `(changed)` when only the blocks below a label did),
  with its `file:line`. Unchanged expressions show no old text. Next and previous go in that order.
- **`expr`** (a value op) sets what vislox's files feed into one input: an expression, written `vx_<uuid>.Port <- …`
  in `edits.loxl` for a block Loxone Config drew and `Port: …` in the declaration of a block of vislox's files, or a
  port of vislox's (`vx_warm.Q`; a bare signal name takes its output), written as a wire and checked as the `wire`
  op checks one (an output into an input, of matching kinds); a signal the request just made is not compiled yet,
  so its output is taken by its expression. `null` drops it, always
  (also when the block is gone or the module manages it now). Refused: an output, an input Loxone Config inverted or
  wired (take that wire out first), one with a constant edited here, a text input, a block the module manages (its
  file), and an operand vislox's files do not name: a block of theirs is named by its slug, one Loxone Config drew by
  `vx_` and its UUID (an extern of `edits.loxl`, added with the expression and dropped with the last name of it), a
  constant by its name; the module's own slugs are not named. lox checks the rest (two-input gates, comparisons do
  not chain, at most four ports in a `Formula`, no units or strings in arithmetic) when the module is checked before
  the compile and when it compiles: what it refuses changes nothing, and its words come back.
- **`signal`** makes, changes or drops a named signal of vislox's file of `page`: `vx_warm = <expression>`, with its
  label as `vx_warm.title = "…"` (none: the expression). The name is `vx_` and lowercase letters, digits and `_`,
  without `__`, not `vx_<uuid>`, and not taken. Changing the top operator (`and` → `or`) is refused: lox keeps a
  pinned block's type; make a new signal and drop this one. Dropping is refused while vislox's files read it (a wire
  or an expression) and once it is deployed (deleting a deployed signal is a later step); a compiled one leaves
  `removed <slug>` for lox, under the comment `# Signals dropped here`, which goes when it is made again or lox
  forgets it. Only such a line is taken back: any other `removed` line (a deleted block) keeps its name taken.
- **Name this expression** sends `signal` (the expression, on the page shown) and `expr` (the input fed from the
  signal) in one request; undone, the expression is back on the input and the signal dropped.
- **Feed an input from an expression** (Properties of a block vislox edits, folded) picks an unwired input and
  types the expression: `expr`. lox mints the blocks and places them on the sink's page.
- **Deleting** a block of vislox's files that an expression of theirs reads is refused (#80's `delete_refusal`).
- Not offered: editing an expression of the module's own files (vislox writes only its own, as for `let`, #81),
  deleting a deployed signal, and an expression on a new block's input in *Add block* (add it, then feed the input).

## Template instances (#83)

A `template` is a block type made of blocks: each instance (`sued = Fassade("Süd", sonne: sonne_sued)`) stamps out a
copy whose lockfile keys are `<instance>_<body name>` (`sued_hoch`; `sued_hitze_alarm` under a nested instance
`hitze`), and source spells those blocks `sued.hoch` and `sued.hitze.alarm`. The compiled config has only the
blocks, so vislox reads the module's source to say which blocks one call made.

```text
template Fassade(label: string, sonne: input, pos: number = 70) -> (Q: b.Q, Hitze: hitze.Q)   # the module's file
	b = And("{label} B", In1: sonne)
	hitze = Waechter("{label} H", quelle: sonne)
end

# Template instances written in vislox: lox makes each one's blocks.               # vislox's page file
vx_ost = Fassade("Ost", sonne: vx_10000006_0000_0060_ffff504f94000001.AQ, pos: 55)
```

- **Read.** `GET /api/edits` sends `templates`: `templates` (each template's `name`, `file`, `line`, `ours` (vislox's
  copy of a library template), `library` (the library file when its text is the library's), `params` (`name`, `kind`:
  `object`, `input`, `value` or `set`, `type`, `default`, `required`), `outputs`, `description` (the comment opening
  its body), `instances` (how many), `deployed` (the deployed revision's files had it) and `changed` (its body as lox
  prints it differs from theirs)); `instances` (`path`: `sued`, `sued.hitze` for a nested one; `prefix`: its blocks'
  lockfile prefix; `template`; `parent`; `label` (a nested call's interpolated with its parent's text parameters, as
  lox's expansion does); `args` as written; the `file` and `line` of its call (a nested one's, its outermost
  instance's, and `body_line` in the parent template); `ours`; `page`; `outputs` (each to its block's UUID and
  connector key, through a nested instance too); `blocks` (`uuid`, `slug`, `name` as source spells it, `expression`:
  lox made it for an expression of the body); `change`: `new` or `call` against the deployed revision's files, for a
  top-level instance); and `library`, lox's template library (`loxl templates`) with each template's parameters. An
  instance of vislox's also carries `edit` (its call as the `instance` op takes it), `revert` (the op that puts the
  deployed revision's call back, or drops it) and `committed`. Read with lox's parser over every `.loxl` file of the
  module, templates classified across files as lox does (`classify_instances_of`). A lockfile key belongs to the
  instance with the longest prefix whose template's body declares the name after it (`<prefix>_<name>`), or for
  which lox made an expression block of that body (`<prefix>_<name>__…`, `<prefix>_<name>_<port>__…`). An
  expression in a body onto an object parameter (`jalousie.AutoShade <- …`) is named after the object passed, not
  the instance, and is no instance's. What the deployed revision had is the module's `.loxl` files at its commit
  (`git ls-tree` and `cat-file`, kept per commit once git read them). Like `sources`, it is the module as it is now: shown on the
  newest config only.
- **The canvas** frames each instance's blocks in a dashed box behind them, headed by `sued: Fassade "Süd"` and its
  arguments and outputs (`sonne: teiler.AQ · pos: 55  → Q, Hitze`); a nested instance's box sits inside its parent's.
  The header's tooltip names the template's file and line, and a click selects the instance's first block.
- **Properties** of a block of an instance has a *Template* section: the block's name as source spells it
  (`sued.hoch`), the instance and its label, the template with its file and line (and whether it is lox's library's),
  the enclosing instances of a nested one, each parameter with its argument or its default, the outputs (linked to
  their blocks), the call's file and line, and *Not deployed* when the template's body changed, the instance is new,
  or its call changed since the deployed revision. One of the module's own files is read-only: *change it there*.
- **The Changes list**, comparing the deployed revision, puts a change to a block of an instance whose template (or
  an enclosing instance's) changed under the template: `Waechter: 2 instances changed · 2 changes`, with the
  template's line; any other change to an instance's blocks goes under its top-level instance, `sued: Fassade (call
  changed)` or `(new)`. The headings come before an expression's (#82) and a constant's (#81).
- **`instance`** makes, changes or drops a template instance of vislox's page file of `page`: `{ "op": "instance",
  "slug": "vx_ost", "page": "<page uuid>", "value": { "template": "Fassade", "label": "Ost", "args": { "sonne":
  "vx_….AQ", "pos": "55" } } }`, written under the file's `# Template instances` comment. `value: null` drops it. Its
  `previous` is the `instance` op that puts back what was (`value: null` when the op made it).
  - The template is one of the module's own, or of lox's library: vislox copies a library template into its
    directory (`<module>/vislox/templates.loxl`, which holds only such copies) for the first instance that calls it,
    and drops the copy with the last one, or when the module declares that name itself. While the module's own
    files do not read, an instance is refused with lox's error rather than taken for the library's template of that
    name. `templates.loxl` is no page's file name (a page titled *Templates* gets `page-templates.loxl`); a page file
    of vislox's that already has the name (it has a `page` line) stays a page file, and a library template is then
    refused (copy it into the module).
  - The name is `vx_` and lowercase letters, digits and `_`, without `__`, not `vx_<uuid>`, not taken, and no other
    name of the module, its lockfile or vislox's files may start with `<name>_` (its blocks are named so).
  - Checked as `lox logic check` checks a call, before anything is written: an unknown parameter (the error lists
    the template's), a missing one (an object, a value or set without a default), a label for a template without
    `label: string`, a value of the wrong kind (`number`, `duration`, `string`, `bool`; a constant's name is taken).
    Each argument is kept as written, so a quoted text stays a text and a constant's name a reference through Save,
    Revert and undo; a text parameter's value typed without quotes is quoted unless it names a constant. An object
    parameter names an object vislox's files may name: one Loxone Config drew as `vx_<uuid>` (an extern of
    `edits.loxl`, added with the instance and dropped with the last name of it), or a block or signal of vislox's, or
    a block of an instance of vislox's (`vx_ost.b`, checked against its template's body; the bare instance is no
    object); an input parameter a port, a bare name or an expression over such names (of an instance: bare for its
    first output, an output or a block); a set parameter a set of vislox's file of that kind. lox checks the rest
    (the object's type, the ports the body uses) when the module is checked and compiled.
  - The template of an existing instance is not changed: lox keeps its blocks' types (make a new instance, then drop
    this one).
  - Dropping is refused while vislox's files read the instance (a wire out of it, an expression, another instance's
    argument) and once any of its blocks is deployed (deleting a deployed instance is a later step: its blocks would
    take #70's removals). A compiled one leaves `removed <instance>_<name>` for each of its blocks, under the comment
    `# Instances dropped here`, which goes when the instance is made again or lox forgets the blocks.
  - Deleting a block or signal of vislox's files that an instance's argument names is refused (#80's
    `delete_refusal`, as for an expression).
- **Add block** lists the module's templates and lox's library's next to the block types (*Template · …*). Picking
  one asks for the instance's name (vislox proposes `vx_<template>_<n>`), its label when it takes one, and each
  parameter (its kind and default as the placeholder): one `instance` op. **Properties** of a block of an instance of
  vislox's edits its arguments (*Save*), puts the deployed call back (*Revert*), and takes out one that was never
  committed (*Remove instance*).
- Not offered: editing an instance or a template of the module's own files (vislox writes only its own, as for
  `let`, #81, and expressions, #82); renaming an instance (`moved` per expanded block); deleting a deployed
  instance; and wiring an instance's output from vislox's `wire` op (feed an input parameter instead).

## Virtual inputs, virtual and Modbus devices (#84)

The objects of the object tree that lox creates are managed blocks with no page and no position: virtual inputs
(`VirtualIn`), virtual HTTP, UDP and text inputs (`VirtualHttpIn`, `VirtualUdpIn`, `VirtualTextIn`) and their commands
(`VirtualHttpInCmd`, `VirtualUdpInCmd`), virtual outputs (`VirtualOut`) and their commands (`VirtualOutCmd`), Modbus
servers (`ModbusServer`), their devices (`ModbusDev`) and those devices' sensors and actuators (`ModbusASensor`,
`ModbusSensor`, `ModbusAActor`). lox appends a device to the base's one folder of its type (*Virtual Inputs*,
*Virtual Outputs*, *Netzwerkperipherie*: `VirtualInCaption`, `VirtualOutCaption`, `WeatherCaption`) and an I/O to
the device its `device:` names, and mints its `IName=` (`VI4`, `VHI1`, `VCI3`, `AMQ12`) once. vislox declares them in
a file of its own, with no `page` line:

```text
# Virtual inputs and outputs and Modbus devices added in vislox: lox puts each       # <module>/vislox/_periphery.loxl
# into its folder or below its device. vislox rewrites this file on every edit;
# see docs/editing.md in vislox.

vx_virtualhttpin_1 = VirtualHttpIn("Zähler", Address: "http://192.0.2.10/status", Poll: 1min)
vx_virtualhttpincmd_1 = VirtualHttpInCmd("Leistung", device: vx_virtualhttpin_1, Check: "\"power\":\\v")
vx_modbusdev_1 = ModbusDev("Regler", device: vx_7e000002_0000_0000_ffff504f94000001)
```

- **Added** with `add_block`, its `page` the folder it goes into or the device it hangs below (`src/periphery.rs`).
  A device of vislox's files is named by its slug; one Loxone Config made by vislox's extern of it (`vx_<uuid>` in
  `edits.loxl`, kept while an I/O names it). It must be in the project's base (lox resolves an extern there); a
  device the module's own files manage is refused (vislox's files name only their own blocks and Loxone Config's).
  The reply's `added` names it with the UUID lox minted, as for a block.
- **Its settings** are lox's attribute parameters of the type (`lox logic types VirtualHttpIn`): text (an address, a
  command, a check pattern, a text input's value, `Desc`), `on`/`off`, or numbers, which take the unit lox documents
  for them (`Poll: 1min` is 60 s, `MaxTime: 46s` is 46000 ms; a plain number is in the saved unit). Strings are
  written escaped (`\` as `\\`, so Loxone's `\v` is `"\\v"`). `IName` and the rest of the element (its
  `<IoData>`, `<Display>`) stay lox's and Loxone Config's: not offered.
- **Checked before lox sees it**: the settings a new one cannot do without (`Address` and `Poll` of an HTTP input,
  `Port` of a UDP input, `Check` of a command, `Address` of a virtual output and of a Modbus server, `ModbusAddress`
  of a Modbus sensor or actuator), where it may go (a command only below its kind of device), no constants and no
  wires when it is added (draw them once it is in the config), and a value a Loxone Config save would rewrite
  (lox's `save_rewrite`: `Poll` is whole seconds from 10 to 604800, a digital virtual input reads 0 to 1, so a
  `MaxVal: 30` needs `Analog: on`). lox's own refusals come back as lox says them.
- **Edited** in Properties as a block of vislox's files is (#79): its name and its settings; a setting lox needs
  cannot be emptied. `Files::ops` lists each (never `device:`, which is where it is), so each is not deployed with
  Revert. Renamed and deleted as #80 says: what hangs below a device follows its rename (lox's `rename_slug`), and a
  device is not taken out or deleted while an object of vislox's files hangs below it, nor while one Loxone Config
  made hangs below it in the pending config or the base (it would be deleted with it; the refusal names them)
  (`blocks[].delete`). A
  deleted one comes back with `restore_block`, whose `page` is its folder or device, checked against what its
  declaration says; its lockfile entry has no layout.
- **Reviewed** like any change: the Changes list and the tree mark it added, with each I/O of a device added with it
  listed too, and a changed setting reads old → new (`Poll: 60 → 120`). Apply deploys it, Discard drops it. Editing
  sends nothing to the device: no test connection, no poll.
- **Not yet**: adopting a virtual input or device Loxone Config made (lox's `adopt --uuid` takes a device with all
  its I/O, under slugs lox picks, `<device slug>_<title>`, into a file per folder; vislox's adoption (#72) moves
  only page blocks into its page files). Until then its settings are Loxone Config's (Properties shows them
  read-only). Operating modes and calendar entries, of the same lox table, are #86's (next section).

## Operating modes and calendar entries (#86)

An operating mode (`Mode`) and a calendar entry (`CalendarEntry`) are objects of the object tree too, declared in
`_periphery.loxl` (`src/calendar_edit.rs`). lox puts a mode into the *Betriebsmodi* folder (`ModeCaption`) and mints
its `Mode=` number once (one below the lowest in use: `-1`, `-2`, …); an entry goes into *Betriebszeiten*
(`CalendarCaption`):

```text
vx_mode_1 = Mode("Grill")                                                              # <module>/vislox/_periphery.loxl
vx_calendarentry_1 = CalendarEntry("Grillfest", switches: vx_mode_1, from: 2026-07-04, to: 2026-07-05)
vx_calendarentry_2 = CalendarEntry("Betriebsurlaub", switches: vacation, every: 08-01)
vx_calendarentry_3 = CalendarEntry("Fasching", switches: "Party", easter: -47)
```

- **A mode** is added with `add_block` (`type: "Mode"`, `page` the Betriebsmodi folder, a name and nothing else).
  It is renamed, taken out, deleted and undone as a block of vislox's files; one is not taken out or deleted while an
  entry of vislox's switches it on (`blocks[].delete` names the entry). A built-in mode (`Mode=` ≥ 0) and one Loxone
  Config made are not edited here (lox adopts a user mode with `adopt --uuid`; vislox's adoption, #72, moves page
  blocks only).
- **An entry** is added with `add_block` (`type: "CalendarEntry"`, `page` the Betriebszeiten folder) and two
  settings: `switches`, the UUID of the mode it switches on, and `date`, as the source writes it. Its mode and date
  are then changed with two value ops, both on an entry of vislox's files only:
  - `entry_mode` (`{ "op": "entry_mode", "uuid": "<entry>", "value": "<mode UUID>" }`): vislox writes the one
    spelling lox takes (`sched_edit::Modes::spell`, as a schedule line): a built-in mode by its day word (`holiday`,
    `vacation`, `dayoff`, `mon` … `sun`, `heating`, `cooling`), a mode the module or vislox names by its slug (not
    a slug that is a day word), any other by its quoted title. A title two modes share is refused.
  - `entry_date` (`{ "op": "entry_date", "uuid": "<entry>", "value": "every: 12-31" }`): one of the four kinds,
    `every: MM-DD` (no year), `easter: <days>` (before Easter Sunday when negative, at most 366 either way),
    `from: YYYY-MM-DD, to: YYYY-MM-DD` (one period, on 2009-01-01 or later, ending on or after its start) and
    `from: MM-DD, to: MM-DD` (every year; across New Year when `to` is earlier). Checked as lox checks it: two-digit
    month and day, a day of the year (`02-29` is one).
  The stored attributes (`CalType`, `Mode`, `Month`, `From`, `EasterOffset`, `WeekDayInMonth`, …) are never
  settings: Properties shows the entry's *When* and *Switches on*, and Advanced the attributes.
- **Undo** carries the mode by UUID, never by its spelling: `remove_block`'s `previous` of an entry is the
  `add_block` with `switches` the mode's UUID. `Files::ops` lists `entry_mode` and `entry_date` for each entry of
  vislox's files, and `edit::current` reads both back from the config (`CalType=` through the modes' `Mode=` numbers;
  the date as the source writes it), so *not deployed* compares the mode and the date, not their wording, and Revert
  works on a deployed entry.
- **Which entries** are edited here is the server's: `GET /api/edits`' `entries` names each calendar entry of the
  config shown, null for an entry of vislox's files, else why not (the module declares it, or Loxone Config made it:
  `lox logic adopt --uuid` takes one over). Each block of vislox's files that is an entry carries `entry` (`mode` by
  UUID, `date` as written) for the form.
- **Read-only**: entries Loxone Config made (adopting them, like adopting its modes, waits for vislox's adoption to
  take objects of the object tree), the module's own entries, and an entry of a weekday in a month (*the last Monday
  of May*), which the language cannot write.

## Tree and Air hardware (#85)

Lox Logic does two things with hardware beyond the settings of #58: it switches statistics on or off for a Tree or
Air input or output, and it creates a Tree device that is not paired yet. Both are hardware declarations, which take
over a device or an I/O ([lox's language](https://github.com/discostu105/lox/blob/main/crates/loxone-config/docs/language.md#hardware--tree-and-air-devices-and-what-hangs-below-them)).
vislox writes them into a file of its own (`src/hw_edit.rs`), with no `page` line:

```text
# Tree and Air hardware vislox declares: …                                              # <module>/vislox/_hardware.loxl

extern vx_hw_20000002_0000_0020_ffff504f94000002 = LoxTree(uuid: "20000002-0000-0020-ffff504f94000002")

vx_presencesensortree_1 = TreeDevice("Melder Flur", kind: "PresenceSensorTree", tree: vx_hw_20000002_0000_0020_ffff504f94000002, branch: "left")
vx_hw_20000003_0000_0030_ffff504f94000002 = TreeDevice(uuid: "20000003-0000-0030-ffff504f94000002")

vx_presencesensortree_1_i1 = TreeSensor(device: vx_presencesensortree_1, iname: "I1", Statistics: on)
vx_hw_20000003_0000_0031_ffff504f94000002 = TreeAsensor(device: vx_hw_20000003_0000_0030_ffff504f94000002, uuid: "20000003-0000-0031-ffff504f94000002", Statistics: on)
```

- **Statistics** (`{ "op": "statistics", "uuid": …, "value": "on" | "off" | null }`): `on` writes the `StatsType=` the
  type records with (7 on a digital input or output, 11 on an analog sensor), `off` removes it. lox takes it on
  `TreeSensor`, `TreeAsensor`, `TreeActor`, `LoxAIRsensor`, `LoxAIRAsensor` and `LoxAIRactor` only (not on an analog
  output); `x.Statistics = on` on an extern is refused, so the I/O is declared, and with it its device. A device
  Loxone Config paired is declared by `uuid:` and states nothing else, so vislox owns no title, room or setting of
  it; an I/O below it by `uuid:` (lox matches `iname:` only inside a device matched by `serial:`). An I/O below a
  device created here is declared by `iname:`, since lox mints its UUIDs anew on every compile until the device is
  deployed. A value op: Revert and Undo as for any value; `null` drops the declaration (and the device's, when no
  other I/O names it), and the I/O records as the base has it. Refused, changing nothing: another type, a word but
  `on`/`off`, an I/O or device the module's own files declare (the refusal names the slug), an I/O not in the base.
  Dropping vislox's claim is always allowed, also when the object is gone since.
- **Shown read-only everywhere**: Properties' *Device* section of a Tree or Air I/O has *Statistics*: `on`, `off`, or
  the other kind Loxone Config set (`StatsType 3`), old → new when comparing. A changed `StatsType=` is reviewed as
  a change of the I/O.
- **A new Tree device**: *Add* on a Tree extension (`LoxTree`) of the project tree is `add_block` with the extension
  as `page`, type `TreeDevice`, a name and the settings `kind` (`TouchTree`, `PresenceSensorTree`,
  `CorridorLightTree`, `ValveTree`: the kinds lox's `tree_kinds` creates) and `branch` (`left`, `right`). It becomes
  `vx_<kind>_<n> = TreeDevice("…", kind: "…", tree: <extern of the extension>, branch: "…")`; lox adds it to the
  extension unpaired (`Serial="00000000"`) with the kind's inputs and outputs (titles in English), keeps its UUID in the
  lockfile (`externals`, `matched_by: created`, and `hardware`), and prints a note on every compile and push. The
  reply's `added` names it with that UUID. The extension must be in the base (lox resolves the extern there):
  `add_under` says so otherwise. Its I/O's statistics are set from Properties of each I/O (by `iname:`); its name
  from its Properties (the declaration's label). Its other values, and those of its I/O (names, rooms,
  settings), wait for Apply: an extern by UUID of an object the base does not have would name nothing, so they are
  refused until then.
- **Marked unpaired**: the project tree tags it *unpaired* (`GET /api/edits`' `blocks[]` entry: `hardware`, `kind`,
  `branch`, `paired` from the config shown), Properties' *Device* section says *Created here, not paired: pair it with
  a Tree search in Loxone Config*, and Apply's confirmation lists it as *added, not paired* (`GET
  /api/apply/changes`' `devices[].unpaired`, for any Tree device added with no serial). Pairing, search and firmware
  stay in Loxone Config.
- **Taken out or deleted** with *Remove*/*Delete* (`remove_block`): its declaration and its I/O's go, with `removed
  <slug>` (lox keeps a created device from its first compile on and refuses to let its declaration go without it).
  Not deployed: its `previous` is the `add_block` with its kind, branch, name and its I/O's statistics
  (`<iname>.Statistics` with the I/O's type, `<iname>.Type`); lox creates it under a new UUID. A designation is letters
  and digits, its slug `<device>_<iname>`. Deployed: Apply deletes it, and its `previous` is the
  `restore_block` (its declaration as the `source`, the extension as `page`; it is taken for a Tree device's by the
  lockfile's tombstone, never by the source), which brings it back with its UUID while lox keeps the tombstone; vislox rebuilds its lockfile entries from the deployed config, checks the tombstone, the
  slug, the kind and the extension, and never takes them from the request. Refused while a wire (drawn in Loxone
  Config or anywhere) leaves one of its objects (lox does not delete a device that feeds a wire), and while vislox's
  file claims a value of one of them. Not offered: renaming it (`blocks[].renames: false`).
- **Not built**: naming a created device's I/O when it is added (lox's English titles until the device is deployed,
  then #58's title claim), adopting a device Loxone Config paired with all its I/O (lox's `adopt --uuid <device>`;
  vislox declares a paired device only by `uuid:`, for its I/O's statistics), and declaring hardware by `serial:`.

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
  - `template`: a block a template instance made (`<instance>_<block>`), at the instance's line. Properties'
    *Template* section names it as source does (`sued.hoch`) and gives the template's line (#83).
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
next push, so the review shows them on their own. vislox reviews them, and edits switching times (#73), access
(#76) and recipients (#77, below).

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

Hooks for the editing issues: #73 (switching times, built) adds the `schedule` op, #76 (access, built) the
`access` op, #77 (recipients, built) the `recipients` op; the review, `set_claims` and Apply's warning needed
nothing more, since they read the compiled config and the module's files.

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

Switching times, moods, access (#76), recipients (#77) and tiles (#78) share one
shape: a named set, claims on blocks, several blocks per set, one set per block. `set_claim.rs` does what is the
same for every kind, parameterised by a `SetKind` (its keyword, block type, content as an op carries it, how
content becomes lines and how lines are checked for each block): resolve the block, choose the set (named, the
block's own when no other block claims it, else `vx_<name>_<stem>`), write it checked for every claimant (refusing
one the config lost), claim it, prune, and build `previous` as a whole inverse (`restore` for every other set the
op rewrote that others still claim). It also builds the section of `GET /api/edits` (`edits_section`: per block
`set`, its content, `deployed`, `shared`, `pending`; per set its blocks, content and `stale` claimants), reading
each config once (`SetKind::index`, `SetKind::differ`), and `set_kinds` tells the browser each kind's keys. The
browser's one Revert (`setRevertValue`) sends the deployed set with its content only while no other block claims
it, else the deployed content in a set of its own; deployed content it cannot read refuses Revert with the reason
(the set's name alone would claim it as it is now, and change nothing).

Every kind of claim on a block's whole content, members too, is one entry of `set_claim::KINDS` (a `ClaimKind`):
reading and rendering vislox's file, the names a new set must not take (the module's names are loaded for any op
of a kind that makes sets), the blocks the rest of the module claims (`Ctx::claimed`, by kind), the refusals of
Remove and Hand back, mending a claim made twice, the check after the compile, and `GET /api/edits` go through it.
A new kind is a `SetKind`, its slot in `Claims`, its `Op` variant, and one line in `KINDS`; #77's
`recipients` and #78's `tiles` were built so. A kind whose lines name other blocks (a tile's targets) gives
`SetKind::lines_with` (the lines and the externs they need, keyed case-insensitively, never retyping another
claim's extern) and `SetKind::named` (the slugs its sets name: kept while named, refused on read unless vislox's,
and named in the refusals of Remove and Hand back).
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
  *Claimed twice* and offers *Remove vislox's claim*. A drop is always let through, also once the module manages
  the block or Loxone Config deleted it: a claim left behind would break every compile (the same holds for
  switching times and moods, `set_claim.rs`).
- **A member added here and not pinned yet** (no UUID in the lock) is carried by its slug in `previous` and in
  `GET /api/edits`, and the op takes the slug back as it is, so its undo works.
- **UUIDs compare lowercase** everywhere (a save may write them either way): the same members in another case are
  no change in Properties, the Changes list or the after-compile check.
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
Ctrl+K palette as a picker (Ctrl+K itself while the editor is shown; otherwise Ctrl+K is the usual palette): only
the blocks of the type the central block takes, not the central block, not one listed already, not one the module
manages (the server's `managed`). A member the config does not have is left out of the draft, which says so. The
draft belongs to the block and the pending config it was opened on: selecting another block, or a new pending
config (an undo, an edit elsewhere), drops it, and *Save* sends that config's blob as `base`, which the server
checks as for any edit. *Save*, *Cancel*, *Revert*, and *Remove vislox's claim* when claimed twice. The Deploy pane counts the central blocks whose members are edited
and not deployed.

## Who may open an NFC Code Touch (#76)

An NFC Code Touch's access rights are a named `access` set and the Code Touch's claim. vislox writes both into
`edits.loxl`, after the externs (and after any `schedule` and `moods` sets):

```text
extern vx_50000009_0000_0090_ffff504f94000001 = NfcCodeTouch(uuid: "50000009-0000-0090-ffff504f94000001")

# Access rights edited in vislox. A claim makes the set everyone who may open the
# Code Touch: whoever it leaves out loses access. Deleting it leaves the rights as last written.
access vx_haustuer_access
	user "alice": "Always"
	group "Bewohner": "Workdays"
end

vx_50000009_0000_0090_ffff504f94000001.access = vx_haustuer_access
```

- **Shown from the compiled config**, as #68 reads it (`rights.rs`): who holds the right directly, who may open it
  in effect, and against the compared revision what changed. Nothing new is read for the view.
- **The op** is `{ "op": "access", "uuid": <Code Touch>, "value": … }`. `value.grants` is the whole list, each
  `{ kind, uuid, schedule }`: `user` or `group`, its UUID and its access schedule's (a `SwitchingTimer`). An empty
  list is a set too: it revokes everyone. `value.set`, `value: null` and `value.restore` are as for moods: write
  vislox's set of that name (for every Code Touch that claims it) and claim it, or with no `grants` claim it as it
  is (*Same rights as …*); drop the claim; write sets back first (an undo's). A new set is
  `vx_<name>_access`. `previous` carries UUIDs (lowercase), never titles, so an undo gives back exactly who held
  the right.
- **Checked before lox sees it**, with lox's rules: an `NfcCodeTouch` only; each user or group once (a set gives
  each one schedule); every title, for every Code Touch on the set, names exactly one user, group or schedule in the
  pending config (lox names them by title: a title two objects share, or none, would not compile, so it is refused
  naming the title, and the pickers do not offer it); at most 500 rights. One set per Code Touch: one the rest of the
  module claims is refused, naming that `file:line` (`set_claims` lists `access` claims), and so is a Code Touch
  the rest of the module manages. Dropping vislox's claim is always let through, as for the other sets.
- **What stays.** Users, groups, schedules, tags and codes stay with Loxone Config and the app. `ACT=` (operating
  the block in the app) is no access right: lox keeps it, and vislox does not touch it. Dropping the claim leaves
  the rights as last written (lox keeps no lock state for it): before Apply, the base's; after, the set's.
- **Checked after the compile.** The pending config must give each Code Touch the request changed, and every
  Code Touch sharing its set, exactly the set's rights (principal and schedule, by UUID, as `rights.rs` reads
  them); otherwise the edit is refused and the files, lock and out are put back.
- **Read back.** vislox reads the sets and claims of `edits.loxl` with lox's parser; an `access` set or claim not
  named `vx_…`, a claim without its set, and a claim on a slug that is neither an extern of the file nor a block of
  vislox's files are refused (a `recipients` set is #77's kind, below). Handing an adopted block back is refused while vislox
  claims its rights.
- **Adopting a Code Touch** (#72): `lox logic adopt` lifts its rights as a set and a claim. vislox leaves both out,
  so the door opens for the same people (a dropped claim leaves the rights as last written); editing them later
  claims them in `edits.loxl`.
- **Who may see it.** `GET /api/edits` sends `access` (per Code Touch vislox's file claims now or claimed in the
  deployed revision: `set`, `grants`, `shared`, `deployed`, `pending`) and `access_sets` (each set with its
  `code_touches`, `grants` and `stale` claimants), by UUID only. The pickers' list, every user, group (with the
  users it reaches) and access schedule by UUID and title, is `POST /api/access/choices` with `{ "base": <blob> }`:
  read from that revision, the one the draft is made on (a blob the History no longer lists is refused with 409,
  never answered from another revision). It needs the admin token (`X-Vislox-Token`, and `X-Vislox`), is sent
  `no-store`, never logged, and the browser keeps it in memory only while the editor is open. No credential is read into it. Names otherwise reach the browser
  only as the review (#68) already shows them, never in a URL, the page title or the event stream.
- **Every change shows in the review and Apply's confirmation** (#68), since both read the compiled config: a
  right taken away directly, through a shared set (*Change … too*, *Same rights as …*), through a group the set
  drops, or because the Code Touch is gone. Apply's confirmation now also gives the schedule a user gains access
  under (*alice gains access to Haustür (Always)*).
- **Fewer opening times are a loss.** vislox cannot tell whether one access schedule's times cover another's, so a
  user who keeps a door but not under every schedule that let them in before (a schedule replaced, or a right
  through a group dropped while another stays) loses opening times. `rights.rs` says so per block (`narrowed`:
  the user, the schedules before and after), and the review, Properties, Apply's confirmation (*⚠ bob may open
  Haustür only at Workdays, no longer at Always*, a warning on its own) and the editor's *Save anyway* warn about it
  like a lost door.

The browser: *Access* in Properties of an NFC Code Touch, on the newest config, with *Edit access*: it asks for
the pickers' list of the config shown (the admin token, the config's blob), then shows a row per user or group (a choice of the config's users and groups
by title, *at*, a choice of its access schedules, ✕) and *Add user or group*. A principal with two rights on the
door (two schedules) keeps the first, and the editor says so. *Save* first names who loses access or opening
times, on this Code Touch and, when *Change … too* is on, on those sharing its set (*Save anyway*), then sends the
op. *Same rights as …* (a Code Touch whose rights vislox claims) does the same: it claims at once when nobody loses
anything, else asks (*Use anyway*). *Revert*, and *Remove vislox's claim* when claimed twice, as for moods. The draft belongs to
one Code Touch and one pending config: selecting another block, or a new pending config, drops it, and *Save*
sends the blob it was made on. The Deploy pane counts the Code Touches whose access is edited and not deployed.

## Push notifications (#77)

A push notification (*Benachrichtigung*, type `Notification`) is made in Loxone Config, under *Messages*. Two things
about it are Lox Logic's: who gets it (a `recipients` set and the notification's claim), and what it says and when
(an `OutputRefLM` block on a page, which mirrors the notification and carries its message). vislox writes both into
its own files:

```text
extern vx_60000010_0000_0100_ffff504f94000001 = Notification(uuid: "60000010-0000-0100-ffff504f94000001")

# Recipients edited in vislox. A claim makes the set everyone who gets the push
# notification (Full access aside): whoever it leaves out stops getting it. Deleting it leaves the recipients as last written.
recipients vx_frost_recipients
	user "alice"
	group "Familie"
end

vx_60000010_0000_0100_ffff504f94000001.recipients = vx_frost_recipients
```

```text
page "Meldungen"

vx_outputreflm_1 = OutputRefLM(mirrors: vx_60000011_0000_0110_ffff504f94000001, On: "Es regnet", MinimumTime: 30min)
```

- **Shown from the compiled config.** `objects[…].message` (`model.rs`) is an `OutputRefLM`'s `<LoggerMailer>`:
  the notification it sends (`notification`, a UUID), the texts sent when its input turns on and off (`on`,
  `off`; none: no message) and the least time between two messages (`minimum_time`, milliseconds).
  `message_old` is the compared revision's when it differs, and such a block is marked changed (the message is
  a child element, which no attribute change shows). The Changes list says it in the language's terms:
  `Frost · message On: "Frost" → "Frost im Garten"`, `… message Off: "…" → —`, `… message MinimumTime: 12h → 30min`.
  Properties of a notification lists the blocks that send it (`mirrored_by`), and its *Recipients* (#68) name the
  users who get it only as members of *Full access* (`rights.inherited`): Loxone Config shows them as inherited
  recipients, writes nothing for them, and no set takes it from them.
- **The `recipients` op** is `{ "op": "recipients", "uuid": <Notification>, "value": … }`. `value.recipients` is the
  whole list, each `{ kind, uuid }` (`user` or `group`, no schedule). An empty list is a set too: only Full access
  gets it then. `value.set`, `value: null` and `value.restore` are as for access (#76): write vislox's set of that
  name and claim it, or with no `recipients` claim it as it is (*Same recipients as …*); drop the claim; write sets
  back first (an undo's). A new set is `vx_<name>_recipients`. `previous` carries UUIDs (lowercase). It is a
  `SetKind` of `set_claim.rs` (`recipients_edit.rs`), with `access_edit.rs`' directory of users and groups, so
  everything #76 says of sharing, *Change … too*, stale claimants, *Claimed twice* and the check after the compile
  holds the same.
- **Checked before lox sees it**, with lox's rules: a `Notification` only (the claim is on the notification, not on
  its block on a page); each user or group once; every title, for every notification on the set, names exactly one
  user or group in the pending config; at most 500. One set per notification: one the rest of the module claims is
  refused, naming that `file:line` (`set_claims`). Dropping vislox's claim is always let through.
- **Checked after the compile.** The pending config must give each notification the request changed, and every
  notification sharing its set, exactly the set's recipients (`ACT=`, by UUID, as `rights.rs` reads them).
- **A push notification on a page** is `add_block` with `type: "OutputRefLM"` and `settings.mirrors` the
  notification's UUID, which must be a `Notification` of the pending config; its message is the settings `On`,
  `Off` (text) and `MinimumTime` (a duration: `12h`, `30min`, `5s`, `500ms`, or seconds). vislox writes `mirrors:`
  an extern of the notification in `edits.loxl`, kept while a block names it. It takes no name (the notification's)
  and no constant: its `AI` is wired from a block once it is on the page (#61), since the parameters take values,
  never wires. Removing it before Apply takes the extern along; its `add_block` names the notification by UUID.
  Its message is edited like any setting of a block of vislox's files (`setting` op on `On`, `Off`,
  `MinimumTime`; empty drops it: no text, no message).
- **Who may see it.** `GET /api/edits` sends `recipients` (per notification vislox's file claims now or claimed in
  the deployed revision: `set`, `recipients`, `shared`, `deployed`, `pending`) and `recipient_sets` (each set with
  its `notifications`, `recipients` and `stale` claimants), by UUID only, and never a message's text. The pickers'
  list is #76's `POST /api/access/choices` (admin token, `no-store`, never logged), whose groups now say whether
  they are *Full access* (`full_access`). Message texts can hold names, phone numbers or mail addresses: they are
  served with the rest of the review, as a block's title is, and never in a URL, the page title, the event stream
  or vislox's log.
- **Every loss shows in the review and Apply's confirmation** (#68): *⚠ Anna stops getting Frostwarnung
  (pages/vislox/edits.loxl:12)*, through a shared set or a group the set drops too. A member of Full access never
  stops getting one.
- **What stays with Loxone Config.** The notification itself (no compile writes into *Messages*), the phones and
  their push settings, and the message of a block Loxone Config drew: lox writes the message only in the
  declaration of a block it manages, and `lox logic adopt --uuid` of an `OutputRefLM` fails in lox 0.83 (it writes
  a label that its own check refuses), so such a block is not adopted here yet. vislox keeps `mirrors:` a value of
  an adopted block's declaration (never a wire), so adopting works once lox does.

The browser: *Message* in Properties of a notification's block, on any revision (the notification, *On*, *Off*,
*Minimum time*, old → new against the compared revision), and *Sent by* on the notification. *Recipients* has
*Edit recipients* on the newest config: the access editor without the schedule column, Full access named (*gets
every notification anyway*); *Save* names who stops getting it first (*Save anyway*), *Same recipients as …*,
*Change … too*, *Revert* and *Remove vislox's claim* as for access. *Add block* lists *Push notification*: the
notification (a choice of the config's), *On*, *Off* and *Minimum time*. Its *Edit values* edits the message. The
Deploy pane counts the notifications whose recipients are edited and not deployed.

## Tiles of a Touch Pure Flex (#78)

A Touch Pure Flex is a Tree device (`TreeDevice`) with a tile layout (`<btns>`): each tile, by its index, has three
functions, the click and the arrows up and down, each a command (`api=`, the app's command language:
`SET(As;Co;pulse)`), a status text (`cT=`) and the blocks the command addresses (`ctrls=`). A command reaches only
the blocks *API-linked* to the device: a block whose `OutputAPI` is wired to the device's API connector
(`ApiActor.I`), directly or through an `OutputRef` of the connector whose `AQ` feeds it (API wires are recorded in
reverse, so either direction counts; `tiles.rs` reads it as lox does). Without targets, a command reaches every
linked block of its type. Lox Logic writes the functions of tiles the layout has:

```text
extern vx_70000001_0000_0000_ffff504f94000001 = TreeDevice(uuid: "70000001-0000-0000-ffff504f94000001")

extern vx_70000012_0000_0000_ffff504f94000001 = AutoJalousie(uuid: "70000012-0000-0000-ffff504f94000001")

# Touch Pure Flex tiles edited in vislox. A claim writes each tile the set lists (its click, up
# and down commands, texts and targets); the others stay. Deleting it leaves the tiles as last written.
tiles vx_flex_kueche_tiles
	tile 2()
	tile 3(
		click: "SET(As;Cc;pulse)",
		up: "SET(As;Co;pulse)",
		to: vx_70000012_0000_0000_ffff504f94000001,
		click_text: "=ECHO(As)",
	)
end

vx_70000001_0000_0000_ffff504f94000001.tiles = vx_flex_kueche_tiles
```

- **Shown from the compiled config.** `objects[…].tiles` (`model.rs`) is a Touch Pure Flex's `tiles` (each `index`
  and its `functions`: `key` `click`, `up` or `down`, `api`, `text`, and `to`, the targets as links, `missing` for
  a UUID the config lacks) and `linked`, the blocks linked to its API connector. `tiles_old` is the compared
  revision's when it differs, and such a device is marked changed (the tiles are child elements, which no
  attribute change shows). The Changes list says it per function: `Flex Küche · tile 3 click: "SET(As;Tg;pulse)" →
  "SET(As;Cc;pulse)"`, `… tile 3 up text: "=ECHO(As)" → —`, `… tile 3 click targets: Nord → Süd`, and a tile the
  layout added or removed. Targets compare by UUID, case aside. A block linked to a device lists the device (`api_linked`).
- **Lint on the tile.** `lox logic lint`'s `tile-broadcast` (a command without targets that two or more linked
  blocks of its type receive) and `tile-dead-command` (one no linked block receives) name the device by its extern,
  or by its title when no extern names it; such a finding is pinned to the Touch Pure Flex of the compiled config
  with that title (`check::pin_tile_findings`), and Properties shows it on each tile its text names (`tile 5`).
- **The `tiles` op** is `{ "op": "tiles", "uuid": <TreeDevice>, "value": … }`. `value.tiles` lists tiles, each
  `{ index, click?, up?, down? }`, a function `{ api, text?, to: [UUID…] }`; a function left out does nothing
  (`tile 3()` clears all three), a tile left out keeps its programs. `value.set`, `value: null` and `value.restore`
  are as for the other set kinds: write vislox's set of that name and claim it, or with no `tiles` claim it as it is
  (*Use the same tiles as …*); drop the claim; write sets back first (an undo's). A new set is
  `vx_<name>_tiles`. `previous` carries the targets by UUID. A target Loxone Config drew gets an extern of vislox's
  file (`vx_<uuid>`, its type), kept while a set names it; a block added or adopted here is named by its slug,
  resolved through the lockfile once lox pins it. One lox has not pinned yet (added in the same request, or carried
  by an undo) is not in the pending config: its type is checked here, its link by lox and after the compile. A
  `tile` is written as lox's formatter writes it (`Module::to_text`), `text:`/`to:` once when every function with a
  command shares them, else per function (`click_text:`, `up_to:`), so `lox logic fmt --check` finds nothing.
- **Checked before lox sees it**, with lox's rules: a `TreeDevice` with a tile layout, for every device on the set;
  each tile index once and one the device's layout has (*it has 2, 3, 5*); a command (one line, at most 500
  characters) on every function given; each target a block of the pending config, once per function, API-linked
  to every device on the set, and of the type a command lox knows the abbreviation of addresses (`As`:
  `AutoJalousie`, `Lico`: `LightController2`; other abbreviations are not checked); at most 64 tiles and 32
  targets a function. A block the module manages is refused as a target (vislox's files name only their own blocks
  and Loxone Config's). One set per device: one the rest of the module claims is refused, naming that `file:line`
  (`set_claims`, which now lists `tiles` claims too). Dropping vislox's claim is always let through.
- **Checked after the compile.** The pending config must have, on each device the request changed and every device
  sharing its set, each tile the set lists exactly as it says (command, text, targets by UUID; an empty function
  for one without a command).
- **What stays with Loxone Config.** The layout (which tiles exist, their place, size and icon), pairing the
  device, and the API link itself: wire a block's `OutputAPI` to the device's API connector in Loxone Config. Dropping
  the claim before Apply leaves the tiles as Loxone Config has them; after Apply, as last written (lox keeps no lock
  state for it).

The browser: *Tiles* in Properties of a Touch Pure Flex (select it in the tree), on any revision: *API-linked*, the
blocks a command can reach (and a linked block's *Touch Pure Flex* section names the device and the tiles that
address it), then each tile with *Click*, *Arrow up* and *Arrow down* (the command, `·` the status
text, `→` the targets), old → new against the compared revision, and the lint findings on the tile. On the newest
config, *Edit tiles* gives each function a command field, a status text field and a checkbox per linked block (a
target the config names that is not linked is listed in red, to take out), and *Clear* per tile. *Save* sends every
tile vislox's set lists and every tile the draft changes, against the pending config the draft was made on; the
draft is dropped when another block is selected or the pending config changes. *Use the same tiles as …*, *Change
… too*, *Revert*, *Remove vislox's claim* (claimed twice) and *Drop its claim* (stale) are as for the other sets.
The Deploy pane counts the devices whose tiles are edited and not deployed.

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
| #76 access | `access vx_… end`, `x.access = vx_…` in `edits.loxl` | An NfcCodeTouch; users, groups and access schedules by a title that names exactly one; several Code Touches may share a set. |
| #77 recipients | `recipients vx_… end`, `x.recipients = vx_…` in `edits.loxl` | A Notification (an extern); users and groups by a title that names exactly one; Full access is never listed. |
| #77 notification on a page | `slug = OutputRefLM(mirrors: vx_…, On: "…", Off: "…", MinimumTime: 12h)` | Mirrors a Notification Loxone Config made; no name, no constant; its `AI` wired with #61. |
| #78 tiles | `tiles vx_… end`, `x.tiles = vx_…` in `edits.loxl` | A Touch Pure Flex (an extern `TreeDevice` with a tile layout); only the indices the layout has; targets API-linked externs or blocks added/adopted here; several devices may share a set. |
| #80 delete a managed block | drop it, `removed slug` | A block added or adopted here; deployed, Apply deletes it on the Miniserver (asking, #70). Undone with its UUIDs while lox keeps the tombstone. |
| #80 rename a managed block | `moved old -> new`, lox's rename over vislox's files | A block added or adopted here; the config does not change. |
| #79 block settings | `slug = Type("Label", At: 06:58, Desc: "…", Code: file("slug.c"))` | Only a block added or adopted here; a program in `<module>/vislox/<slug>.c`. Constants take a unit (`Time: 30min`, `x.Time = 1h`). |
| #81 constants | `let vx_… = 30min` in `edits.loxl`; `x.Time = vx_…`, `Time: vx_…` | Made, changed and dropped in vislox's file; a port bound to it, or to a constant of the module. The module's own `let` and `defaults` lines are shown, not changed. |
| #85 statistics | `vx_hw_… = TreeSensor(device: …, uuid: "…", Statistics: on)` in `_hardware.loxl` | A Tree or Air input, digital output or analog sensor; its device declared by `uuid:` alone. Not on one the module declares. |
| #85 new Tree device | `vx_<kind>_<n> = TreeDevice("…", kind: "…", tree: …, branch: "…")`, `removed vx_…` | The four kinds lox creates, on a Tree extension of the base; unpaired until a Tree search in Loxone Config. |
| #86 operating mode | `vx_mode_<n> = Mode("…")` in `_periphery.loxl` | A new user mode; lox mints its `Mode=` number. Built-in modes and Loxone Config's stay as they are. |
| #86 calendar entry | `vx_calendarentry_<n> = CalendarEntry("…", switches: …, every: 12-31)` in `_periphery.loxl` | A new entry, then its mode (`entry_mode`) and date (`entry_date`, one of four kinds). Loxone Config's entries stay read-only. |

Blocks the module already manages (they are in the lockfile) are refused: they are edited in their own `.loxl`.
The blocks vislox added or adopted (#72) are the exception: their `.loxl` is vislox's.

## What lox cannot do, so vislox does not offer it

- Parameters stored on the block element of a block the module does not manage: a Formula's text, a Daytimer's
  On/Off labels, a memory flag's type, a virtual input's Analog, MinVal, MaxVal and DefVal, and `Desc` on logic
  blocks. lox takes them only in the declaration of a block it manages: adopt the block first (#72), then edit it
  as a managed block (#79, see *Block settings*). Properties shows them read-only until then.
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
    devices and their I/O: objects of the object tree, added from the project tree instead (#84, see *Virtual
    inputs, virtual and Modbus devices*); operating modes and calendar entries, added in the Operating modes
    view (#86);
  - the NFC Code Touch, Intercom and Touch Pure Flex controller blocks, which exist because Loxone Config paired
    the device (lox adopts them, never creates them);
  - blocks lox creates but that do nothing until they are set up in Loxone Config: the burglar and smoke alarms,
    the room controller, the irrigation, and the central light and shade blocks (the members of one the config
    has are edited here, #75);
  - every type outside lox's table (lox refuses it: "Refuse, never guess"). A template is added as an instance
    (#83).
- On a new block: a wire when it is added (draw it once it is on the page, #61) or an expression on an input (feed
  it once the block is on the page, #82),
  a position when it is added (lox places it; it can be moved after), and a room or category (lox has no claim for
  them on a managed block). Its settings (`Formula:`, `Type:`, `At:`, a program, …) are given when it is added and
  changed later in Properties (#79).
- A block on a page whose title an earlier page has, and a new page (#88).
- A push notification itself (lox never writes into *Messages*), and adopting a notification's block Loxone Config
  drew (`lox logic adopt` of an `OutputRefLM` fails in lox 0.83): its message is changed in Loxone Config (#77).
- A Touch Pure Flex's layout (adding, moving or removing a tile, its icon) and its API link: lox writes only the
  functions of tiles that exist, and a target must already be linked (#78). A block the module manages as a tile's
  target: vislox's files name only their own blocks and Loxone Config's, so such a tile is programmed in the
  module's own file.

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
| #76 (built) | NFC Code Touch access (`access`, `.access`) |
| #77 (built) | notification recipients and messages (`recipients`, `OutputRefLM`) |
| #78 (built) | Touch Pure Flex tiles (`tiles`, `.tiles`) |
| #79 (built) | a managed block's label and settings (`Desc:`, `Type:`, `Formula:`, `Code: file(…)`, …), constants with units |
| #80 (built) | delete and rename managed blocks (`removed`, `moved`) |
| #81 (built in part) | `let` constants made and changed in vislox's file, ports bound to them; the module's `let` and `defaults` shown |
| #82 (built in part) | expressions (`<-`, `Port: …`) and named signals (`slug = …`, `.title`) of vislox's files; the module's shown |
| #83 (built in part) | `template` instances of vislox's files (the `instance` op, lox's library copied in); the module's shown, grouped and reviewed |
| #84 (built in part) | virtual inputs, virtual and Modbus devices added and edited in `_periphery.loxl` (`device:`); adopting Loxone Config's is left |
| #85 (built) | Tree and Air hardware: `Statistics:` on an I/O, new Tree devices (`kind:`, `removed`) in `_hardware.loxl` |
| #86 (built in part) | operating modes and calendar entries added in `_periphery.loxl`, an entry's mode and date edited (`switches:`, `every:`, `easter:`, `from:`/`to:`); adopting Loxone Config's is left |
| #87 | Automatic Designer rules (`rule`) |
| #88 | new pages (`page "…" (room: …)`) |

Around the module: #64 (mark managed blocks, built), #65 (`lox logic check` errors, built), #66 (lint, built), #67 (`lox logic test`, built),
#68 (review claimed access, recipients and schedules, built), #70 (removals on Apply, built: a block the module
lost is deleted only with consent to exactly that list, see [features.md](features.md#apply-sync-and-discard)).

## The browser

- Properties has an *Edit* section on the newest revision: *Edit values* opens a form for the name, the
  room and category, the unwired inputs and the settings. Numbers are checked in the browser; lox checks ranges and the rest.
- *Add* on a folder of the project tree (*Virtual Inputs*, *Virtual Outputs*, *Netzwerkperipherie*) or on a virtual
  or Modbus device opens the same dialog for the types that go there (`GET /api/block-types`' `periphery`: each type's
  `folder` or `devices`, and its settings with their unit and whether a new one needs them), the settings lox needs
  first; it is off, with the reason, where an edit would be refused (`GET /api/edits`' `add_under`: each folder and
  device that takes new objects, null or why `add_block` there would be refused) (#84). A number takes a unit only
  where lox documents one (`Poll: 1min`, not `Port: 30min`), and is range-checked in that unit. The new object is selected in the tree.
  A Tree extension takes a new Tree device the same way, its kind and branch picked from lists (#85).
- *Edit values* on a Tree or Air input or output has *Statistics* (on or off) where lox switches them; below a Tree
  device created here and not deployed it is the only field (#85).
- The Operating modes view has *Add mode* and *Add entry* in its header, and *Edit* on each calendar entry (off, with
  the server's reason, on one Loxone Config made) (#86). The form takes the name, the mode it switches on (the
  config's modes) and the date by kind. A click on a day of the year picks it (for *Relative to Easter*, its offset
  from that year's Easter Sunday), a drag across days picks a period (or the range every year, for that kind), and the
  draft's days are outlined on the year. A draft is dropped when the pending config changes, and an entry's when it
  is no longer selected; saving sends the blob it was opened on. *Add* on the project tree's calendar folder opens
  the same form (the palette's `view: "modes"`); the Betriebsmodi folder's *Add* adds a mode with its name.
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
- A block of vislox's files shows its slug in the Edit section with *Rename* (a field: Enter renames, Escape
  cancels; the draft closes when the selection or the pending config changes) and, once deployed or adopted,
  *Delete* (asks first; off with `blocks[].delete` as the reason). The Deploy pane lists the `removed` and `moved`
  lines lox needs no more, with *Drop them* (#80).
- *Arrange*, next to *Add block*, lays out the page's managed blocks afresh. It is off, with the reason, where
  the server would refuse it (`arrange` in `GET /api/edits`).
- A refused move puts the block back where it was. A move that compiled stays drawn even when showing the new
  revision fails; Properties says so.
- `GET /api/edits` lists what vislox's files claim now and in the deployed revision, which of it is not
  deployed, the blocks vislox added (`blocks`: slug, UUID, type, page, file, and whether the deployed config has
  it), the other blocks the module manages (`managed`), where the module makes each managed block (`sources`,
  #64), the module's `let` constants and `defaults` and the ports that take a value from one (`constants`, #81), its expressions and named signals with their blocks (`expressions`, #82), the blocks lox can move (`movable`: managed and drawn on a page) and
  those the pending config draws elsewhere than the deployed one (`moves`: UUID, position, deployed position),
  for each page with such a block whether *Arrange* can lay it out (`arrange`: page UUID → null, or why not), and
  the wires vislox's files draw now or drew in the deployed revision (`wires`: `from`, `to`, `now`, `deployed`), the
  wires the rest of the module draws (`module_wires`) and those an adopted block brought along
  (`adopted_wires`), why vislox leaves those and Loxone Config's alone (`wire_why`), and which blocks can be
  adopted (`adopt`, #72). It
  is worked out again only when vislox's files, the lockfile, the deployed or
  the pending config changes (`actions.edit.key`); the browser fetches it only then.
