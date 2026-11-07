# Changelog

vislox follows [semantic versioning](https://semver.org). Release tags are `vX.Y.Z`; the Docker image is
published under the same version (see [docker/README.md](docker/README.md)).

## Unreleased

- Edit values in Properties (a `lox.toml` project, with the admin token): a block's name, room and category, the
  constant of an unwired input, and the settings of a physical input or output or a device. vislox writes them
  into its own file of the module (`<module>/vislox/edits.loxl`) and runs `lox logic compile`; they show as the
  pending revision, marked *not deployed*, with Revert. Editing never deploys (#58, see `docs/editing.md`).
- Add blocks to a page: *Add block* on the page picks one of the types lox can create (searched by name and
  purpose), then its name, settings and constants. vislox declares it in a file of its own for the page
  (`<module>/vislox/<page>.loxl`) and runs `lox logic compile`, which mints the block and places it. It shows in
  the pending revision and the Changes list as added, is edited in Properties like any block, and has *Remove*
  until it is deployed. Never deploys (#59).
- Move blocks: drag a block the module manages on the page (or Alt and an arrow key), snapped to Loxone Config's
  grid, its wires following. lox keeps a managed block's position in the lockfile, so vislox writes it there and
  runs `lox logic compile`; the move shows in the pending revision as *moved*, with Revert. *Arrange* lays out a
  page's managed blocks afresh (`--relayout`). Blocks Loxone Config drew, and input and output references, are
  not moved: lox writes no position for them. Never deploys (#60).
- Draw wires: drag from a port to a port on another block (an output to an input), drag an end of a wire drawn
  here to reconnect it, and click a wire for its card with Delete. vislox writes each wire as `to <- from` into
  `<module>/vislox/edits.loxl` and runs `lox logic compile`; it shows in the pending revision and the Changes list
  as added or removed, with Revert. A link Loxone Config would not have (an input to an input, text into a number,
  an inverted input, a pair already wired, a connector vislox cannot tell the kind of) is refused with the reason. A wire Loxone Config drew cannot be taken
  out: lox removes only the wires its source draws. Never deploys (#61).
- Undo and redo the edits (#62): *Undo* and *Redo* next to *Add block* (Ctrl/⌘+Z, Shift+Ctrl/⌘+Z), each
  naming the edit in its tooltip, and a list of recent edits. An undo is sent as an edit of its own (the reply of
  every edit now carries `undo`, the ops that undo it), so it is compiled and checked like any edit and never
  deploys. The history is cleared, saying why, when the pending config changes outside the page (another edit, a
  compile, Discard, Apply, Sync). It is the tab's own, kept for the config it was made on, so it survives a reload.
  A relayout is undone by one new op, `place`, however many blocks the page has.
- Adopt blocks Loxone Config drew into the module (#72): *Manage with Lox Logic* in Properties (with the slug it
  gets) and *Adopt all* on a page run `lox logic adopt` on a copy of the module and lockfile; vislox moves the
  declaration into its file of the page (its neighbours as externs pinned by UUID), adds the block to the
  lockfile, compiles, and refuses unless the config is exactly what it was, each block exactly the base's (same
  UUIDs, connectors, wires, parameters and place). An adopted block is then moved, renamed, given constants and
  settings, and wired like a block added here. *Adopt all* says why each block it skips stays Loxone Config's.
  The ops are `adopt`, `adopt_page` and `unadopt` (the undo, also *Hand back*). Never deploys; nothing is pending
  after it, and it goes into git with the next Apply.
- Roll back an Apply (#69): in the History, the deployed Apply has *Roll back to before this* when its push left
  the download from before it in `backups/`. vislox shows what that deploys (the module and lockfile from before
  the Apply, compiled onto that download, which must give it back) as a revision compared with what runs now,
  asks with Apply's confirmation, then runs Apply's steps: the tests, `lox config push --force --patched-from
  <the Apply's post-push download>` (which refuses unless the Miniserver still runs exactly that), the program
  restart, verification, a commit (`Roll back: <subject>`) and git push. The in-place compile, lox's staged upload
  (`lox config push` without `--force`, first) and its verified download must each be the program reviewed. It
  refuses on any doubt: changes not committed, a later deploy, a download that is missing, unreadable or two
  different programs, an untracked or ignored file in the way, a module that does not give it back. Files git does
  not track are never touched. A push that fails after its upload is judged by what the Miniserver runs (recorded
  as deployed, put back, or reported as unknown); before it, the project is put back.
- Removed managed blocks on Apply (#70): a push deletes every block its lockfile tombstones (a block the module
  lost, which lox compiles only with the removal allowed, or one a compile tombstoned before). Apply used to fail
  with lox's error, or delete them without asking. The Deploy pane now says so and offers *Review the removals*:
  vislox compiles the module with `--allow-removals` in a scratch copy, changing nothing, and shows the result as
  the revision *Removals allowed*, compared with what runs, with each block it deletes (name, type, page, what is
  wired to it, references on other pages, who loses access or a notification with it, #68). Apply asks *Remove N
  blocks from the Miniserver?* with exactly that list, then shows its full confirmation for that program; also
  when lox refuses an Apply for it. The consent names the program and its blocks; Apply compiles exactly that into
  the project (`lox logic compile --allow-removals` with the plan's `--time`), puts the lockfile and `out` back on
  any failure before the push goes through, and pushes without `--allow-removals`. A push that fails after its
  upload is judged by what the Miniserver runs, as for a Roll back (kept as deployed, put back, or unknown). Without consent Apply refuses,
  also for a tombstone already in the lockfile. A retry asks again; a Roll back takes no consent.
- No personal data in git (#71): before every git push vislox makes (Apply, Roll back, Sync), the new step *No
  personal data in git* runs `lox logic scan` on each config file version a push to the branch's upstream would
  send (`.Loxone`, `.LoxCC`, a backup `.zip`'s program; the whole repository), and then pushes exactly that commit
  (`git push --no-follow-tags <remote> <sha>:<branch>`: no other branch or tag). A finding stops the push: the commit
  stays local (an Apply has deployed, and says *Applied, but not pushed to git*), and the Deploy pane's *Not pushed
  to git* lists each finding by file, line, kind and element, never the value, with what to do (take the file out
  of git) and **Push to git**, which scans again and pushes (`POST /api/push`). It fails closed: a file lox cannot
  read, a lox without `logic scan`, an answer vislox cannot read, or a branch without an upstream push nothing. A failed `git push` offers *Push
  to git* too, and so do commits not pushed when vislox starts. A project that tracks a real base config is
  blocked until the base is git-ignored.
- Edit a Daytimer's switching times (#73): *Switching times* in Properties of a DayTimer opens an editor with a
  day bar per operating mode (the seven weekdays, and the holiday, vacation, day off, heating, cooling and the
  config's own modes it adds): click the bar to add an interval, drag it to move it, its ends to resize it (15
  minutes, Alt: one), and type exact times, a value on an analog timer and *activate* below; *Copy to* weekdays,
  weekend or every day. *Save* sends the new op `schedule` (columns by their mode's UUID), which writes a Lox
  Logic `schedule vx_… end` set and the timer's `.schedule` claim into `<module>/vislox/edits.loxl`, grouped into
  `daily`, `weekdays` and `weekend` lines, and compiles; the compile must write exactly those switching times onto
  the timer. Several timers share a set (*Use the same times as …*, and *Change … too* when editing one of them);
  Undo restores every timer the op changed (a rewritten shared set too); Revert changes only its own timer;
  Discard drops it all. A timer the module's own file claims too is marked *Claimed twice*, with *Remove
  vislox's claim*. Checked before lox sees it: whole minutes, `24:00` ends a day, no
  overlap, no wrap past midnight, a value on each interval of an analog timer and none on a digital one, one
  schedule per timer (a claim of the module's own file is named instead). A Miniserver that runs other switching
  times (changed in the app) is named in the editor: the save, once applied, replaces them. Never deploys.
- Show, review and edit a lighting controller's moods (#74): Properties of a LightController2 has *Moods*: its
  circuits (`AQ1` … `AQ18`), each a switch, dimmer, colour or tunable-white circuit and named after its actuator
  or marked *no actuator* (with `lox logic lint`'s `no-actuator` finding on the circuit), and its moods in order
  with their values in Lox Logic's terms (`AQ1 on, AQ2 80%`), the off mood, the built-in and motion moods and the
  T5 input that selects each marked. The Changes list has a line per changed, added or removed mood
  (*Wohnzimmer · mood "Lesen": AQ1 on, AQ2 80% → AQ1 on, AQ2 40%*) and a new order. *Edit moods* adds, renames,
  reorders and deletes moods and sets each circuit's value, offering only the values its kind takes; *Save* sends
  the new op `moods`, which writes a Lox Logic `moods vx_… end` set and the controller's `.moods` claim into
  `<module>/vislox/edits.loxl` and compiles; the compile must write exactly those moods. Checked before lox sees
  it: names (one each, never the off mood), values each circuit takes, "Viel Licht" and the moods the motion
  settings name kept. Deleting or renaming a mood a button selects is warned about first. Several controllers
  share a set (*Use the same moods as …*, *Change … too*); Undo restores every controller the op changed; Revert
  changes only its own; a controller the module's own file claims is changed there (*Claimed twice*: *Remove
  vislox's claim*). On a controller added or adopted here, a circuit is declared a switch or a dimmer
  (`Circuit<n>:`), re-checked against the moods vislox claims. A shared set whose other claimant Loxone Config
  deleted is refused by name, with *Drop its claim* (switching times too). Switching times and moods share one
  implementation of set claims (`set_claim.rs`), ready for access, recipients and tiles. Never deploys.
- Show, review and edit the members of a central block (#75): Properties of a CentralLight or CentralShade has
  *Members*: the lighting controllers or automatic blinds it drives, in the order the app lists them, each a link
  to its block (one the config lacks is named by its UUID). An alarm's members (`Objects=`) are shown read-only:
  they stay with Loxone Config. The Changes list reads a changed list by name (*Alle Lichter · member added:
  Diele*, *… member removed: …*, *… members order: …*) instead of the raw `rec=` line. *Edit members* adds one
  with the Ctrl+K picker (only blocks of the type the central block takes, none listed already), removes and
  reorders them; *Save* sends the new op `members` (the whole list, in order; `[]` empties it), which writes a Lox
  Logic `<central>.members = (…)` claim into `<module>/vislox/edits.loxl` and compiles; the compile must write
  exactly that list. Checked before lox sees it: the right type, each listed once, never the central block itself,
  no block the module manages, one claim per central block (one the module's own file makes is named, *Claimed
  twice*: *Remove vislox's claim*). Undo, Revert and the Deploy pane count it like the other edits; a member
  Loxone Config deleted is named, with *Drop the missing members*. Adopting a central block leaves its list as
  Loxone Config wrote it. Never deploys.
- Edit who may open an NFC Code Touch (#76): *Access* in Properties has *Edit access*: a row per user or group
  with its access schedule (pickers of the config's users, groups and access schedules by title, asked for with
  the admin token from the new `POST /api/access/choices`; a title two objects share cannot be picked), ✕ and
  *Add user or group*. *Save* sends the new op `access` (users and groups by UUID, each with its schedule's UUID),
  which writes a Lox Logic `access vx_… end` set and the Code Touch's `.access` claim into
  `<module>/vislox/edits.loxl` and compiles; the compile must give the Code Touch exactly those rights. Who loses
  access is said before the save (*Save anyway*), and Apply's confirmation says it again, now with the schedule a
  user gains access under; a user who keeps a door under fewer access schedules is warned about as a loss
  everywhere (review, Properties, editor, Apply). *Same rights as …* shares a set between
  Code Touches; *Change … too* rewrites a shared set, its undo gives the others theirs back. Built on
  `set_claim.rs`. `ACT=` (operating it in the app) is kept. Credentials never leave the server; `GET /api/edits`
  names users, groups and schedules by UUID only. Adopting a Code Touch leaves its rights as they are. Never deploys.
- Push notifications (#77): Properties of a notification's block on a page (`OutputRefLM`) has *Message*: the
  notification it sends, the *On* and *Off* texts and the minimum time between messages, marked against the
  compared revision; a changed text or time reads old → new in the Changes list (`Frost · message On: "…" → "…"`).
  A notification lists the blocks that send it, and *Recipients* names the members of Full access as inherited.
  *Edit recipients* is the access editor without schedules: the new op `recipients` writes a `recipients vx_… end`
  set and the notification's `.recipients` claim into `edits.loxl` (a `SetKind`, shared sets, undo, Revert, the
  Deploy pane), warns who stops getting it before the save (*Save anyway*) and in Apply's confirmation with ⚠.
  *Add block* offers *Push notification*: an `OutputRefLM` that `mirrors:` a notification Loxone Config made, with
  its message (`add_block` with `settings.mirrors`); its input is wired by dragging (#61). Its message is edited in
  Properties (*Edit values*: On, Off, Minimum time as a duration). Never deploys.
- Touch Pure Flex tiles (#78): Properties of a Touch Pure Flex has *Tiles*: the blocks linked to its API connector
  (directly or through an OutputRef) and each tile by index with its click, arrow up and arrow down (command, status
  text, targets as links), marked against the compared revision; a changed function reads old → new in the Changes
  list (`Flex Küche · tile 3 click: "…" → "…"`, `… click targets: Nord → Süd`). lox logic lint's `tile-broadcast`
  and `tile-dead-command` show on the tile (pinned to a device lint names by title). *Edit tiles*: the new op `tiles`
  writes a `tiles vx_… end` set (in lox's own format) and the device's `.tiles` claim into `edits.loxl`, its targets
  picked from the linked blocks as externs; checked as lox would (indices the layout has, API-linked targets, `As`
  and `Lico` types) and after the compile. A `SetKind` (shared sets, undo, Revert, the Deploy pane); the module's
  own `.tiles` claims are listed in `set_claims`. Never deploys.
- Fixed: Revert of switching times, moods or access whose deployed content could not be read sent the set's name
  alone, which claimed the set as it was and changed nothing; it is now refused with the reason. One Revert
  serves every set kind, from the keys the server sends (`set_kinds`).
- Fixed: Discard failed when the last commit had no lockfile (the first compile writes one), or one was staged
  but not committed. Discard now reads the last commit, never deletes a lock vislox did not write (one not in git
  goes back to how it was before vislox's first edit), and puts everything back when a step fails.
- Fixed: Discard and the operate buttons ignored mouse clicks.
- Block settings (#79): Properties shows the settings a block keeps on its element (a PulseAt's time and pulse, a
  memory flag's type, a Formula, the name the app shows, a meter's units, a program block's program) by their Lox
  Logic names and as Lox Logic writes them, marked against the compared revision; the Changes list reads them the
  same way (`At: 18:58 → 06:58`, a program as a line diff). Constants read with their unit (`Time 30min`). On a block
  added or adopted here they are edited in *Edit values*: a program in a text area, written into a file of its own
  (`<module>/vislox/<slug>.c`, `Code: file(…)`); constants take a unit (`1h`, `250ms`). On a block Loxone Config
  drew they are read-only until it is adopted. Never deploys.
- Delete and rename managed blocks (#80): a block added or adopted here is renamed in Properties (*Rename*: lox's own
  rename over vislox's files and `moved <old> -> <new>`, so it keeps its UUIDs and the config does not change) and,
  once deployed or adopted, deleted (*Delete*: `removed <slug>`; the review shows it removed, Apply deletes it on the
  Miniserver after the removals are confirmed, and Undo brings it back with its UUIDs while lox keeps the
  tombstone). The Deploy pane offers to drop the `removed` and `moved` lines lox needs no more. Never deploys.
- Constants and defaults (#81): Properties names the `let` constant or the `defaults` statement a parameter takes
  its value from (`Time 30min (nachlauf)`), with its file and line, and the Changes list puts the changes one
  constant causes together under it. *Make a constant* names a value in vislox's file (`let vx_… = 30min`) and binds
  the port to it; another block takes it by its name. Changing it asks: the constant (every port bound to it) or
  only this block. The module's own `let` and `defaults` lines are shown, not changed: vislox writes only its own
  files. Never deploys.

## 0.2.0

- Which config the Miniserver runs: the program file it loaded, from `def.log` and `/prog`, under the History
  title, with warnings for an upload still waiting (`sps_new.zip`), the emergency config (`Emergency.LoxCC`),
  and a newest file the Miniserver skipped or does not run (#28).
- Apply verifies by load: after the program restart, `sps_new.zip` must be gone and `def.log` must show it was
  loaded. Otherwise Apply says "Applied, but not verified" and why (#28).
- vislox has a version: `vislox --version` and the UI show it with the commit it was built from. A release tag
  must match `Cargo.toml`.
- Apply follows the program restart step by step, then shows the change running (#35).
- Miniserver status chip, and a preflight before Pull, Apply and Sync that checks the serial (#36, #37), and a
  notice when the config changed after the base (#39).
- Verified TLS under the CloudDNS name, and Remote Connect (#52).
- History: the Miniserver's own backups between Pulls (#53).
- Changes that reconfigure hardware, with the device settings explained (#54).
- Miniserver log panel (#40) and, opt-in, its Debug-Monitor (#46).
- Live values: opt-in queries for the blocks behind a page's wires (#45), clearer close codes and back-off (#34).
- What the Loxone app shows (#43), calendar entries and operating modes as a year calendar (#41, #47, #48),
  Daytimer schedules and weather (#50), heating and cooling periods (#51).
- Operate virtual inputs from the canvas (opt-in: `--operate`) (#42).
- Softer wires and clearer history clicks (#56); a shorter README with details in `docs/` (#57).
- Removed: the experimental Change with AI section (#33).

## 0.1.0

The first version, before releases were tagged: the Loxone Config-style viewer with history, change review,
Pull from Miniserver, live values, Change with AI, Apply, Sync and Discard, and the Docker image.
