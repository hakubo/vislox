# Features

What vislox shows and does, in detail. For setup and flags see the [README](../README.md); for every request vislox
sends to the Miniserver see [miniserver.md](miniserver.md).

## Viewing a config

- **Pages as tabs**: one sheet tab per logic page along the bottom, as in Loxone Config. Each tab shows how
  many changes it holds.
- **Blocks, refs and wires** in the Loxone Config 16 look: rounded blocks with a coloured header, grey port
  stubs, constants beside unwired inputs, and pill-shaped references with their address and an analog (`#`) or
  digital (◉) mark. Block footprints and port rows come from `loxone_config::layout`, the same geometry
  `lox` uses to place blocks. Input and output refs show as reference bars with their short address
  (`VI1`, `AQ1`, …). Wires have rounded corners and hop over the wires they cross. Each wire's vertical
  run takes its own lane, clear of blocks and other wires; wires from one output share theirs and branch.
- **Links between pages**: when a wire's far end is on another page, the port gets a reference bar with
  the far block, its port and its page. Click it to go to that page. Double-click a ref tag to go to the
  block it mirrors.
- **Memory flags and their references** (#104): a memory flag (Loxone Config's *Merker*) is one object with a
  write side (its input) and a read side (its outputs), and Loxone Config shows it by side: on its own page as a
  tag for each side in use, and on other pages as an *output reference* (writes it) and an *input reference*
  (reads it). vislox draws it that way. A tag's pill says the way through it: **→M** for the side the flag is
  written through, **M→** for the side it is read through (a reference of a flag says the same: an output
  reference **→M**, an input reference **M→**). A flag wired on both sides of a page is two tags, one at the input
  and one at the outputs, with a gap between them; a flag wired on one side is one tag at that side; a flag wired
  on neither is a plain **M** pill. A reference's own wire to the flag it shows is not drawn (it is plumbing; the
  tag is the reference). A flag wired to a block on another page gets a link at that port, like any block.
  Properties of a flag, an output reference of it or an input reference of it has *Memory flag*: **Written**
  (the blocks wired to its input and the output references of it, each a link to the page) and **Read** (the
  blocks wired to its outputs and the input references of it), or *nowhere: nothing sets it* / *nothing uses it*.
  A reference's section also names the flag it *Writes* or *Reads* (a link). Live values: a flag passes on a value
  only when one source sets it (a wire to its input, or an output reference, or both for one source); with two
  sources it holds whichever wrote last, which no wire says, and shows no value unless the Miniserver streams it.
- **Sidebar with the block's configuration**: the periphery and page tree (filter with Ctrl+F), and the
  selected block's properties: name, type, page, room, category, connections (each one a link),
  parameters and raw attributes.
- **Calendar entries as dates**: an operating-mode calendar entry shows *When* in words (*Every year on 24 Dec*,
  *Easter Monday (Easter +1)*, *4 Jul 2026 – 5 Sep 2026*, *Every year 1 Oct – 30 Apr*) and the mode it *Switches
  on*, linked to the `Mode` object. The tree's tooltip says the same, and the Changes list reads a changed entry
  as *24 Dec → 26 Dec* and *Holiday → Party* instead of raw attributes. An entry vislox cannot decode says why;
  its raw attributes stay under Advanced.
- **Automatic Designer rules** (#87): Properties of a rule (`AutopilotRule`) has *Rule*: active or not, the rule in
  one sentence (*On weekdays and at 09:00: Pumpe: on*), then each condition (*When*, the groups split by *or*) and
  each action (*Then*) in order, with a link to the block it watches or drives or the operating mode it names. It is
  read as Lox Logic reads it (lox's decompile), so states and commands are the app's own names. A rule Lox Logic cannot
  write (text values, calls, speech output) says so; its payload stays under Advanced. Compared with another revision,
  the Changes list reads a changed rule per line (*Pumpe Ein · condition changed: at 09:00 → at 10:00*, *· action
  added: wait 5s*, *· active → inactive*) in place of the raw `APC=` change.
- **Operating modes** (Ctrl+K *Operating modes*, or **Year** on the tree's Calendar or Operating modes folder): a
  year calendar of when each mode is on. Each day carries the colour of every mode its entries switch on; the year
  buttons and ← / → outside the date grid change years (from 2009, Loxone's day 0), and a mode's chip hides or
  shows its days. Each date is a
  keyboard accessible button named with its full date and the modes and periods on it; focusing it shows the same
  day details as hovering. Tab enters at one day; the arrow keys move by day or week, Home and End move to the
  week edges, and PageUp and PageDown change the year. Enter or Space selects the first entry just as a click does. Clicking a day or an entry in the
  list beside the year selects the entry (Properties). Today is outlined. With live
  values on, the modes that are on now (their `Q`) are marked *on now*, and the chip's tooltip says whether the
  calendar has that mode today or logic switched it on. In change review, days the viewed revision adds are
  outlined green and days it removes red (amber for both), so a timespan in the wrong year stands out. An entry whose date no longer decodes shows its old days as removed. Easter
  comes from the Gregorian computus; a yearly range that ends before it starts runs across New Year. Weekday
  entries (*the last Monday of May*) are not decoded yet, so they are not on the calendar.
- **Heating and cooling periods** (live values on): the Miniserver's heating and cooling periods, which Intelligent
  Room Controllers and the Climate Controller use to decide whether to heat or cool. They are set in the app's
  operating-mode settings, not in the config, so vislox reads them from the Miniserver
  (`jdev/sps/calendargetheatperiod` and `calendargetcoolperiod`, read-only) each time the live connection
  opens, which includes after the program restart of a deploy. A period that is not read (no answer within 10 s,
  a refusal) is asked for again after 10 s, 30 s and then every 5 minutes; until then the last connection's value
  shows, marked *from the previous connection*, and a failed read never replaces a good one. The Operating modes view shows *Heating period: 1 Oct –
  30 Apr · Cooling period: 1 Mar – 7 Sep* with a chip each (it hides or shows the period's days), marks their days
  with a red and a blue bar, and says whether today is in either period. An Intelligent Room Controller's or Climate
  Controller's Properties list the same under *Heating and cooling*, with a link to the year. A period the
  Miniserver does not give (another code, no answer, or an answer that is not `MM-DD/MM-DD`) says why, and so
  does the view while live values are not connected (offline, stopped). Not with `--live-from`.
## History and change review

- **History** (right): every revision, newest first. The config on the Miniserver is marked *on Miniserver*: the program the last Apply, Roll back or Pull left there
  (a commit that failed after an Apply, or one made by hand, does not move it); after a restart, the newest commit;
  without git, the file on disk. In a git repository with no commit of the file it is not known, and nothing is
  marked. An edit on disk that nothing committed is the file vislox watches, not the deployed config: it is not
  marked, and says *not in git*. *Pending* is compared with the deployed revision, not with the newest one.
  Click an entry to show it; Shift+click another to compare with it:
  - *Pending*: the compiled `out.Loxone` of a `lox.toml` project, compared with the deployed `base`.
    This is what `lox config push` would apply.
  - *Changed on disk*: snapshots taken while vislox runs, whenever the file changes.
  - *Git commits*: the file's history, e.g. from `lox config pull`.
  - **Pull from Miniserver** (in a `lox config init` repository): runs `lox config pull`, which downloads the
    Miniserver's current config and commits it when it changed. The new backup appears at once.
  - **Which config the Miniserver runs**, under the History title: the program file it loaded (e.g.
    `Runs sps_0274_20261106003351.zip · fw 17.2.10.21`), read from its system log (`def.log`) and the files in
    `/prog`. `lox config pull` and lox's verify take the newest `sps_<ver>_<timestamp>.zip`, but the Miniserver
    loads by a longer order (`Emergency.LoxCC`, `sps_new.zip`, the newest dated file its firmware can load, then
    `sps.zip` …), so a warning shows when `sps_new.zip` is still waiting (an upload that was never loaded), when
    it runs its emergency config (after several reboots in a row), or when the newest file was skipped because it
    needs a newer firmware or Pull would download another file than the one it runs. Without access to `def.log`
    (it needs an administrator) the line says *Probably runs* and follows the load order. ↻ checks again; a
    check is kept for a minute.
  - *Miniserver backups*: the Miniserver keeps a backup of each config it loads (`/prog/sps_<ver>_<stamp>.zip`;
    one Miniserver kept its last 20). **Read** lists them and downloads each new one once (FTP, read-only). A backup with the
    bytes of the Pull next to it shows as that Pull (a *Miniserver backup* pill); the others are listed under the Pulls they
    were saved between (*Between Backup v273 and Backup v274*, *Since the last Pull*). Click one to show it, and
    *Compare with the revision before* (Changes) compares it with the save before it. So every save from Loxone Config can be reviewed on its own, even when nobody pulled in
    between, and shift-click compares any two.
- **Change review**: added (green), changed (amber), removed (red, dashed ghost) and moved (blue, ghost at
  the old place) blocks. Wires and parameters are marked too: changed values show old → new in the
  properties. The Changes list can be filtered by kind, and clicking an entry jumps to it.
- **Changes that reconfigure hardware**: a change to an extension or a Tree, Air or bus device (its settings, its
  serial, adding or removing it) makes the Miniserver send the device a new configuration, which the device
  stores itself. Those changes carry a *hardware* tag, and the *hardware* chip next to the kind filters shows only
  them. A new serial reads as *device replaced: 0x8003 Touch Tree, serial A → B*. Properties has a *Device*
  section: the kind and its bus type code, the device an input belongs to, what the change does to the device, and
  the settings matched to the device's own configuration, each with a tooltip on what it does there (*Threshold,
  V: the AI Extension only sends a new value when …*). Apply's confirmation says *This reconfigures N devices*
  and warns when a new serial is not on the bus; after Apply, *Changed by this Apply* lists the reconfigured
  devices and whether they are online, for three minutes.
- **Readable at every zoom**: zoomed out, each block shows only its name, at a readable size. From about 80 %
  the full detail appears. Gates show their operation (`AND`, `Less`, `Divide`) instead of Loxone's automatic names
  (`O208698`); a named gate keeps its operation in grey, as Loxone Config 16 shows it. Selecting a block highlights its wires and neighbours and dims the rest.
- **Blocks the Lox Logic module manages** (a `lox.toml` project): a block whose UUID the lockfile pins
  (`logic.lock.json`, or `lock` in `lox.toml`) has a purple λ badge at its upper left; an `extern` stays unmarked,
  since Loxone Config owns it. Its Properties have a *Lox Logic* section: the slug, the `.loxl` file and line
  that make it (`pages/beschattung.loxl:12`) and *Copy path*, for pasting into an editor or an agent's prompt. A
  block lox made for an expression (`temp_hoch and sonne` becomes an `And`) points at the line of the expression;
  one a template instance made, at the instance. The blocks of one expression are drawn as one block showing the
  expression (`markise.In2 ← licht.Q and teiler.AQ >= 28`); a click opens the group (a dashed frame titled with the
  expression, whose title folds it again), and Properties of any of its blocks has an *Expression* section: the
  expression, what it feeds (or the named signal and its label), and its file and line (#82). A template instance's
  blocks are framed together, headed by the call (`sued: Fassade "Süd"`, its arguments and outputs; a nested
  instance inside its parent's frame), and Properties of any of them has a *Template* section: the block as source
  spells it (`sued.hoch`), the instance, the template with its file and line, the arguments and outputs (#83). The legend's *Show managed blocks only* fades the rest, and
  Ctrl+K finds a block by its slug as well as its name. The lines follow the module's files as they are on disk,
  compiled or not, and are shown on the newest config only (the lockfile is today's). See [editing.md](editing.md#blocks-the-module-manages-64).
- **Navigation**: Ctrl+K finds any page or block (a managed one by its slug too). There is a minimap, a "Changed pages only" filter, and links
  (`#page=…&sel=…&rev=…`) that reopen the same page, block and revision. Back and Forward restore the page, selection, revision and comparison from each URL. A URL without a revision follows the latest view and keeps its comparison with the revision it is based on.
- **Keyboard**: the tree, tabs and canvas all work from the keyboard; press `?` for the shortcuts.

## Live values and the app

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

  - **Query blocks on this page** (in the legend, off by default): vislox asks the Miniserver for the outputs
    of the blocks behind the page's wires that it does not stream (`jdev/sps/io/<uuid>/all`, a read-only
    request), every 2 s, at most 20 requests a second, on the connection live values already use. Wires that
    pass through references are followed to the first block behind them (a memory flag or a mode answers for
    itself). These
    values have a dotted outline and green tint and say "queried" in the tooltip: they are up to one round
    old (a page with more than 40 blocks takes longer than 2 s). The legend says how many of this page's blocks
    answered and which cannot be queried (an alarm or an NFC Code Touch answers `500`; a block that does not
    answer three times in a row is left alone for a minute). Queries stop when you leave the page, and their
    values go with them.

  Without queries, blocks with timing or memory (delays, edge detection, Formula, …) and inputs the app does
  not show stay without a value. Those wires are faded ("Dim wires without a value" in the legend turns this
  off). Values show on the deployed revision only (see History: what the last Apply, Roll back or Pull left on the
  Miniserver). A save on disk that nothing deployed is not it, so on it live values stay off.

- **In the app**: what a block looks like in the Loxone app. Properties shows whether the config shows it in the
  app, in which room and category (changes marked old → new; showing, hiding or moving a block to another room or
  category counts as a change),
  and, from the Miniserver's structure file (`LoxAPP3.json`), the control type and the icons the app uses. Blocks
  shown in the app carry a blue phone on the canvas (legend: *in the app*): only they and the periphery have live
  values, and a block that is not in the app says so. The sidebar tree shows rooms and categories with the app's
  icons. On the revision the Miniserver runs, a block the config shows that the app lacks, or has in another room
  or category, gets a warning; after Apply, **Changed by this Apply** runs the same check on the changed blocks
  (and on removed blocks the app still shows) against the first structure file read after the Apply started.

- **Schedules, weather and Status blocks**: a Daytimer's switching times (and an Intelligent Room Controller's
  heating and cooling schedule) show in Properties as a day bar per operating mode, with the default value. A
  changed schedule reads per mode in the Changes list (*Uhr · schedule, Party: 18:00–23:00 22.5 → 17:00–23:00
  22.5*). With live values on, the bar of what the Miniserver runs sits under the config's, and modes that differ
  are marked: on the deployed revision as *changed in the app since this config*, on *Pending* as what Apply would
  replace. Apply's confirmation lists the schedules it would change that way (`GET /api/schedules?rev=…`), or says that
  it could not check. A Status block shows its live text
  with the app's colour (and its icon, when the structure file lists it), and a weather data block's output carries the weather server's current
  value (with a weather subscription; the Weather Service block lists them all).

## Apply, Sync and Discard

- **Apply, Sync, Discard** (for a `lox.toml` project, when started with an admin token): edit the project's Lox
  Logic module and compile it with `lox logic`, by hand or with an agent of your own working in the repository
  (see [agent-workflow.md](agent-workflow.md)). The compile shows as the pending revision, so you
  review it like any other change. **Apply** first checks that the module compiles and its tests pass, then
  deploys it with `lox config push --force` (download, drift check,
  upload, program restart, verify), then commits the project and, once `lox logic scan` finds no personal data in
  what the push would send (#71, below), pushes it to its git remote. Each step shows
  as it happens, with how long it took. vislox follows the program restart itself: the Miniserver going out of
  service, back online after N s, and the new config loaded (its structure version changed). When Apply
  finishes, Apply also checks **by load**: `sps_new.zip` must be gone from `/prog`, and `def.log` must show a
  new program start that loaded it and runs the newest backup. Otherwise it reports *Applied, but not verified*
  and why (it still commits and pushes). Then the view switches to the deployed revision, compared with the one it replaced, and turns live
  values on. **Changed by this Apply** lists the changed blocks with their first live values. **Discard** puts
  the module back to the last commit. **Sync from Miniserver** makes what the Miniserver runs now the project's
  base (`lox config merge`: changes saved from Loxone Config, rules edited in the app), then commits and pushes;
  it waits until nothing is pending. When merge refuses because the Miniserver holds custom changes no download
  carries, Sync makes a plain download the base instead (rules edited in the app are then not taken over).

- **Roll back** (#69; the same project and token): `lox config push` keeps what the Miniserver ran before each
  push (`backups/<stamp>-pre-push.zip` and `.Loxone`, git-ignored) and what it verified after
  (`<stamp>-post-push`). A commit whose base is a post-push program is a push, and the deployed one gets **Roll
  back to before this** under its History entry; older pushes show the button off (*only the newest deploy can be
  rolled back*), and a commit no kept download belongs to gets none. A Roll back is an Apply, with nothing skipped:
  - First vislox works out what it deploys, changing nothing (`POST /api/rollback/plan`, while no other action
    runs, holding the compile lock): the module and lockfile as git had them before the commit, compiled onto the
    pre-push download in a scratch directory (`lox logic compile --base … --module … --lock … --out …`). That
    must give back the download (byte for byte, or no difference lox's diff sees), else it refuses and names the
    differences. The result shows as the revision *Roll back: <subject>* (`rollback:<commit>`), compared with what
    runs now, in the browser that asked only; Cancel forgets it (`POST /api/rollback/cancel`), and the server drops
    it after 15 minutes or once another commit is deployed. The confirmation is Apply's: who loses or gains access
    and notifications (#68), the switching times the Miniserver runs differently, calendar entries changed in the
    app, the devices it reconfigures, a config saved after the base, and the number of changes.
  - It refuses on any doubt: changes in lox.toml, the base, the module, the lockfile or out that are not
    committed (Apply or Discard them first), any of them changed in a commit after the one rolled back, a later
    deploy, a download that is missing, unreadable, or two programs (the `.zip` and the `.Loxone` differ), a
    commit with nothing before it, a file git does not track (an ignored one) where the module from before has
    one, or a module that does not compile onto the download. Files git does not track in the module are never
    deleted or overwritten: only tracked files go back.
  - Then the same preflight as Apply (serial, FTP, web port), and the plan worked out again: it must be the
    program you reviewed (the browser sends its blob id). Apply's steps follow, each checking that what goes up
    is what you reviewed: *Backup checked*; *Module before the Apply compiles to it*: written into the project
    (lox.toml's base pointed at a copy of the download in `backups/`, the base file untouched) and compiled there
    with the project's own settings (`lox logic compile`), `out` must be the reviewed program; *Tests pass*
    (`lox logic test`, #67); *Upload is what you reviewed*: `lox config push --patched-from
    backups/<stamp>-post-push.Loxone` without `--force` runs lox's drift check (the Miniserver must still run
    exactly what the Apply deployed; a save from Loxone Config since stops it) and stages the upload, which must
    extract to the reviewed program. Only then the same command with `--force`: custom changes made in the app
    are deleted only when you allow it, as for Apply; lox uploads, restarts the program and verifies by download,
    and that download must be the reviewed program too. vislox follows the restart, verifies by load, keeps the
    base in git, commits `Roll back: <subject>` (its body names the download and the commit) and pushes.
  - When something fails: before lox's upload line nothing was sent, and the project is put back as it was.
    After it, vislox downloads what the Miniserver runs (`lox config download`, read-only) and compares: the
    rolled-back program is recorded as deployed (committed, *Rolled back, but not verified*); the Apply's program
    with no upload waiting in `/prog` means nothing changed (put back); anything else (no answer, a third
    program, `sps_new.zip` waiting) is reported as unknown, with the project left at its last commit and how to
    find out (*Which config the Miniserver runs*, then Pull or Sync).
  - Afterwards the project matches the Miniserver: the base is what it runs, the module and lockfile are as before
    the Apply, nothing is pending, and **Changed by this Roll back** lists what changed. So neither Sync nor
    Discard is needed. To try the change again, edit the module again, or roll back the Roll back (it is a push
    too, and deploys the Apply's config again).
  - A program restart loads the upload, as for Apply; lox's `--reboot` is not used.

- **Removed managed blocks** (#70; the same project and token): a push deletes every block its lockfile
  tombstones. lox refuses to compile a module that no longer has a block its lockfile manages, unless the removal
  is allowed (`removed <slug>` in the module, or `--allow-removals`, which deletes *every* block the module lost),
  and either way the compile tombstones it. vislox deletes none of them without asking:
  - The check finds them (a scratch compile of the module as it is, and the project's lockfile): lox's refusal
    for a block the module lost, or the blocks the push would delete. The Deploy pane says so, with lox's words or
    the blocks, and **Review the removals**.
  - vislox works out what Apply would deploy then, changing nothing (`POST /api/removals/plan`, while no other
    action runs, holding the compile lock): the module compiled with `--allow-removals` and a fixed `--time` onto
    a scratch copy of the lockfile. The blocks are that compile's tombstones for blocks the Miniserver runs, and
    the managed blocks it drops that were never deployed. The result shows as the revision *Removals allowed*,
    compared with what runs, and the Deploy pane lists each block: its name, type and page, what is wired to it
    (from which block and port, to which), its references on other pages, or that it was never deployed. The
    server keeps one plan, for 15 minutes: a newer one (another tab's) replaces it, cancelling forgets it, and a
    tab's preview goes with it.
  - Apply asks first: *Remove N blocks from the Miniserver?*, with exactly that list and who loses access or a
    notification with them (#68). Then Apply's own confirmation, about the plan's program: what changed on the
    Miniserver since, rights, switching times, calendar entries, devices. Declining either forgets the plan. When
    lox refuses an Apply the check had not caught, the browser asks both again, for the plan, before it sends the
    Apply once more with the consent.
  - The consent names the plan's program (its blob id) and its blocks. Apply's *Removals you confirmed* step
    compiles in the project (`lox logic compile --allow-removals --time <the plan's>`): `out` must be the reviewed
    program and the blocks it removes the confirmed ones, else the lockfile and `out` are put back and Apply
    refuses. The tests run on that compile, the module and lockfile must still be it before the push, and
    `lox config push` runs **without** `--allow-removals`: lox deletes the tombstoned blocks, and refuses a block
    the module loses after the compile. **Until the push goes through, any failure** (the tests, a refusal to
    accept a Loxone Config version or delete custom changes, the push before its upload line) **puts the lockfile
    and `out` back**, so no tombstone outlives the request: a later Apply asks again. A push that fails after lox's
    upload line is judged as a Roll back's (#69, the same code): vislox downloads what the Miniserver runs
    (`lox config download`, read-only). The program you confirmed: it is kept and committed as deployed, *not
    verified*. The previous program with no upload waiting in `/prog`: nothing changed, put back. Anything else
    (no answer, a third program, an upload waiting): reported as unknown, the lockfile and `out` put back, and how
    to find out (*Which config the Miniserver runs*, then Pull or Sync).
  - Without consent, the step refuses, with the blocks or lox's words, before the tests: also for a tombstone
    already in the project's lockfile. With nothing to remove it is skipped. A Roll back never takes a consent.

- **No personal data in git** (#71; the same project and token): a real config holds the installation's address,
  its users' names, mail addresses, phone numbers and access codes, credentials and hardware serials. When a
  `.gitignore` misses one (a downloaded `live.Loxone`, a backup outside `backups/`), a commit takes it along. So
  before **every** git push vislox makes (Apply, Roll back, Sync, *Push to git*), Apply's step *No personal data in
  git* asks `lox logic scan --json` about what the push would send:
  - Where: the branch's upstream (its remote and branch there). Without one, or on no branch, vislox refuses and
    says how to set one: it never guesses a remote. A commit counts as pushed once the upstream's tracking branch
    has it: the scan, *Push to git* and the check on start all use that.
  - What: exactly the objects `git push` sends for `HEAD` (`git rev-list --objects <sha> --not <the remote's
    tracking branches>`; the whole repository, not only the project), and of those every file that may hold a
    config: `.Loxone`, `.LoxCC`, and a Miniserver backup `.zip` (its `sps0.LoxCC`). Each version: a file a commit
    adds and the next deletes is still sent. A version the remote has already (a merge of what it has) is not
    sent, so not read. lox reads copies in a scratch directory made readable to this user only (mode 700, files
    600, never widened later), removed afterwards. The whole scan has 5 minutes.
  - Then exactly that commit is pushed: `git push --no-follow-tags <remote> <sha>:<branch>`, so no other branch or
    tag goes along, whatever `push.default`, a push refspec or `push.followTags` say, and a commit made during the
    scan stays for the next push (*Not pushed to git yet*).
  - lox names where personal data is: the element and attribute and the value's length, never the value. vislox
    adds the line it finds that element on and the commit, and never reads a value itself, so neither its log, the
    browser, nor an error message can show one.
  - A finding blocks the push: *No personal data in git* fails, *Pushed to git* is skipped, and nothing is pushed.
    The commit stays local: after an Apply or a Roll back the Miniserver runs the new config and the commit records
    it, so the History, Discard and Roll back still match it; **only the git remote is behind**, and Apply says so
    (*Applied, but not pushed to git*). The Deploy pane's *Not pushed to git* lists each finding as `file:line: kind —
    element attribute (N chars)` (and the commit, for a version deleted since) and what to do: take the file out of
    git (`git rm --cached`, a line in `.gitignore`, `git commit --amend` for the last commit, `git rebase -i` for
    an earlier one), then **Push to git**, which scans again and pushes (`POST /api/push`, with the admin token;
    nothing goes to the Miniserver). lox has no allowlist: a file that must stay in git must not hold personal data
    (`lox logic scan --redact` writes a copy without it).
  - **Fail closed**: a file lox cannot read (a `.zip` that is none, a backup whose program cannot be read), a lox
    without `logic scan` (older than 0.83), an answer vislox cannot read, no upstream (or one not fetched), or a
    scan over 5 minutes: nothing is pushed, and the box says why. A push with no config file in it needs no lox.
    When `git push` itself fails (credentials, the remote), the box says so too, with *Push to git*; after a
    restart, commits the upstream does not have offer it again (*Not pushed to git yet*). The 3 s git poll looks
    again when `HEAD` or the upstream moves: pushed from a terminal, the box goes and Apply's steps say *Pushed to
    git*; so they do after *Push to git*. The box lists at most 50 findings and counts the rest.
  - A project that tracks its deployed config (vislox copies the verified download over the base, below) is
    blocked as soon as that base holds personal data, which a real one does. Keep the base and `out` out of git
    too (`git rm --cached`, `.gitignore`), as lox keeps `backups/`: the commit then holds the module and lockfile.
  - `lox config pull` (Pull from Miniserver) commits the full config by design and pushes nothing: it is not
    scanned.

- **Does the module compile?** (a `lox.toml` project): vislox runs `lox logic check` on the module whenever it
  changes. While it has errors, `out.Loxone` stays at the last compile that worked, so the Deploy pane says *The
  module doesn't compile: N errors*, lists each one with its file, line and message, and links the blocks its line
  names (Properties says it too). Apply is off, and the server refuses it, until the module compiles. When the
  module compiles but to something other than the pending config (edited outside vislox and not compiled), it
  says *Pending is from before the latest edit to `pages/x.loxl`* and offers **Compile** (`lox logic compile`, with
  the admin token; nothing is deployed). Files `lox logic fmt --check` would format are a hint, never a blocker.
  How: [editing.md](editing.md#when-the-module-does-not-compile-65).

- **Lint** (a `lox.toml` project): with the same check, while the module compiles, vislox runs `lox logic lint` on
  the project: unused externs, constants and templates, managed blocks whose outputs feed nothing, Touch Pure Flex
  commands that reach several blocks or none, values outside a port's range, bare times that read better with a
  unit, and the rest of lox's kinds. Each block a finding is about gets an amber warning badge on the canvas (its
  tooltip says why) and the finding in Properties' *Lox Logic* section. The Deploy pane's *Lint: N findings* lists
  them all by kind, each with its file and line and a button per block that shows it on its page. An NFC Code
  Touch's Properties list the users whose NFC tag or code opens no Code Touch. Findings never stop Apply. How:
  [editing.md](editing.md#lint-findings-66).

- **Tests** (a `lox.toml` project): with the same check, vislox runs the module's `test "…" … end` blocks in
  lox's simulator (`lox logic test`). The Deploy pane's *Tests: N of M passed* lists each test with pass, fail or
  couldn't run, the `expect` that did not hold with its expected and actual value, and buttons for the blocks it
  checks; a test that could not run names the blocks the simulator does not model, so a pass isn't read as full
  coverage. A block a failing test checks gets a red badge, and the test in Properties. **Apply is off while a test
  fails**, and Apply's *Tests pass* step runs the tests of the exact config it would deploy and stops there,
  before anything is sent. How: [editing.md](editing.md#tests-67).

- **Rights and schedules** (#68): who may open an NFC Code Touch, who gets a push notification and a Daytimer's
  switching times are on no logic page, yet a Lox Logic claim owns them (`haustuer.access = bewohner`,
  `frost.recipients = familie`, `uhr.schedule = abend`). While a change is compared, *Rights and schedules* above
  the change list shows per block the users and groups added, removed (red) or given another access schedule,
  who loses or gains the right in effect (through groups too), and for a Daytimer the old and new week side by
  side; each with the `file:line` of the module's claim. The change list has them under *rights*. Properties of a
  Code Touch (*Access*) or a notification (*Recipients*) lists who holds it, who it opens for or is sent to, and
  *Managed by `pages/access.loxl:12`*; a Daytimer's *Schedule* names its claim too. Apply's confirmation warns
  first: *⚠ alice loses access to Haustür (pages/access.loxl:12)*, and names the claim of a schedule the
  Miniserver runs differently. Names show only in the review: never in a URL, the page title or the log. How:
  [editing.md](editing.md#rights-and-schedules-a-change-claims-68).

- **Edit values** (for a `lox.toml` project, with the admin token): Properties' *Edit values* changes a block's
  name, its room and category, the constant of an unwired input (lox checks it against the documented range) and the settings of a
  physical input or output or a device. *Save* writes them into vislox's own file of the module and runs
  `lox logic compile`, so they show as the pending revision; nothing is deployed until Apply. Each value saved and
  not deployed says *not deployed*, with *Revert*; Discard drops them all. Blocks the module manages are edited in
  their own `.loxl`. How it works, and what comes next: [editing.md](editing.md).

- **Add blocks** (the same project and token): *Add block* at the top left of a page lists the block types lox can
  create on a page, searched by name, type and purpose, with Loxone's description. Pick one, then give it a name,
  its settings (a Formula's text, a memory flag's type, a PulseAt's time, …) and the constants of its parameters
  and inputs; what you leave empty gets the value Loxone Config places the block with. *Add* declares it in
  vislox's file for the page and runs `lox logic compile`, which places the block. It shows as added in the
  pending revision and the Changes list; its Properties edit its name and constants, and *Remove* takes it out
  again until it is deployed.

- **New pages** (the same project and token, #88): *New page*, next to the page tabs, asks for a title, a room
  (required: Loxone Config needs one) and a category, from the rooms and categories of the project's base. vislox
  writes a file headed `page "Küche Logik" (room: "Küche", category: "Beleuchtung")` and runs `lox logic compile`,
  which creates the page. Its tab shows in the pending revision marked added, and blocks added there go into the
  same file. *Remove page* takes it out again while it is not deployed and empty; Undo creates it again. Once
  deployed it is Loxone Config's: rename or move it there.

- **Constants and defaults** (the same project and token, #81): a parameter whose value a `let` constant or a
  `defaults` statement of the module gives says so in Properties (`Time 30min (nachlauf)`, `ModbusPollingCycle 60
  (defaults ModbusASensor)`), with the statement's file and line and how many ports take it. Comparing the newest
  revision, the Changes list puts the parameter changes one constant causes under it (`let vx_nachlauf = 45min · 2
  changes`). In *Edit values*, *Make a constant* names a field's value (`let vx_… = …` in vislox's file, the port
  bound to it); another block takes it by its name, typed into its field. Changing a value one of vislox's constants
  gives asks: *Change the constant (N uses)* or *Only this block*; so does one of the module's own constants, whose
  value vislox rewrites on its line and nothing else (refused when the line changed since the page showed it).
  *Default…* on a managed block's parameter makes or changes a `defaults` statement: it lists every block the default
  changes, the module's own among them, and the edit names exactly those. The Deploy pane and Apply's confirmation
  name each changed constant and default with the blocks it changes. How:
  [editing.md](editing.md#constants-and-defaults-81).

- **Expressions and named signals** (the same project and token, #82): an expression of vislox's own files is
  edited in Properties' *Expression* section: its text, and *Name this expression*, which makes it a named signal of
  the page (`vx_warm = …`) and feeds the input from it; a signal's label too. *Feed an input from an expression*
  gives an unwired input of a block vislox edits an expression of its own. lox makes the blocks and the compile says
  what it refuses. The Changes list reads an edited expression as its old → new text, not as blocks added and
  removed. The module's own expressions are changed in its files. How:
  [editing.md](editing.md#expressions-and-named-signals-82).

- **Template instances** (the same project and token, #83): *Add block* lists the module's templates and lox's
  template library next to the block types; picking one asks for the instance's name, label and arguments, and
  writes the call into vislox's page file (a library template is copied into vislox's directory while an instance
  calls it). Properties' *Template* section edits an instance of vislox's: its arguments, Revert, and *Remove
  instance* while it was never committed. The arguments are checked as `lox logic check` checks a call. Comparing the
  deployed revision, the Changes list puts the changes a template's body caused under the template (`Waechter: 2
  instances changed`), and those of one call under its instance (`sued: Fassade (call changed)`). The module's own
  instances and templates are changed in its files. How: [editing.md](editing.md#template-instances-83).

- **Virtual inputs and devices** (the same project and token, #84): *Add* on the *Virtual Inputs*, *Virtual
  Outputs* and *Netzwerkperipherie* folders of the project tree, and on a virtual or Modbus device, adds a virtual
  input, a virtual HTTP, UDP or text input, a virtual output, a Modbus server, a command, a Modbus device or its
  sensor or actuator: a form per type with its settings, the ones lox needs first. vislox writes it into
  `<module>/vislox/_periphery.loxl` (`device:` names where an I/O hangs); lox mints its designation (`VI4`, `VHI1`).
  Properties edits its name and settings; Rename, Delete and Undo work as for a block. The Changes list shows it added
  (with each command of a device added with it) and a changed setting old → new. Nothing is sent to the device.
  Adopting one Loxone Config made is not offered yet. How: [editing.md](editing.md#virtual-inputs-virtual-and-modbus-devices-84).

- **Operating modes and calendar entries** (the same project and token, #86): the Operating modes view's *Add mode*
  adds a user mode by its name (lox numbers it), and *Add entry* a calendar entry: its name, the mode it switches on
  and its date, one of four kinds: every year on a day, relative to Easter, one period, a range every year. Click a
  day of the year, or drag across days, to pick the date; keyboard Enter picks a day and Shift+Enter extends a
  range from the first picked day. The draft's days are outlined, and once compiled the new
  days show outlined as added. *Edit* on an entry added here changes its name, mode and date; Undo, Revert, Remove
  and Delete work as for a block, and a mode is not deleted while an entry switches it on. vislox writes
  `<module>/vislox/_periphery.loxl` and names each mode the one way lox takes it (`holiday`, a slug, or `"Title"`).
  Entries and user modes Loxone Config made become editable after **Adopt into vislox** in Properties.
  Built-in modes and weekday-in-month entries stay read-only. Editing never changes the Miniserver's live calendar. How: [editing.md](editing.md#operating-modes-and-calendar-entries-86).

- **Tree and Air hardware** (the same project and token, #85): *Edit values* on a Tree or Air input, digital output
  or analog sensor switches its *Statistics* on or off (shown read-only on every revision, old → new when comparing).
  *Add* on a Tree extension of the project tree creates a Tree device lox can create (Touch Tree, Presence Sensor
  Tree, LED Ceiling Light RGBW Tree, Valve Actuator Tree) on the left or right branch, unpaired: the tree tags it
  *unpaired*, Properties and Apply's confirmation say to pair it with a Tree search in Loxone Config. It is taken out
  or deleted with Remove/Delete, and Undo brings it back (with its UUID once deployed); one with a wire is not
  deleted. vislox writes `<module>/vislox/_hardware.loxl`. Pairing, search and firmware stay in Loxone Config. How:
  [editing.md](editing.md#tree-and-air-hardware-85).

- **Delete and rename managed blocks** (the same project and token, #80): Properties of a block added or adopted
  here shows its slug with *Rename* and, once deployed or adopted, *Delete*. *Rename* rewrites the slug in vislox's
  files with lox's own rename and adds `moved <old> -> <new>`: the block keeps its UUIDs and the config does not
  change (vislox checks). *Delete* drops the declaration and adds `removed <slug>`: the review shows the block
  removed, Apply deletes it on the Miniserver after you confirm the removals (as any removal), and Undo brings it
  back with its UUIDs until then. Once compiled, the Deploy pane offers to remove the obsolete statements (lint's
  `stale-lifecycle`). How: [editing.md](editing.md#deleting-and-renaming-managed-blocks-80).

- **Draw wires** (the same project and token): drag from an output to an input of another block (or the other way
  round). The ports it may end at light up; one it may not (an output, a text port for a number, an inverted
  input, a pair already wired) says why. The wire is written into vislox's file of the module and compiled, and
  shows as added in the pending revision. Drag an end of a wire drawn here to reconnect it; click a wire for its
  card, with *Delete* (or the Delete key) and *Revert*. A wire Loxone Config drew is removed in Loxone Config.

- **Adopt blocks** (the same project and token, #72): Properties of a block Loxone Config drew offers *Manage with
  Lox Logic*, with the slug it gets (`vx_…`, from its name if left empty). *Adopt* runs `lox logic adopt` on a
  copy of the module and lockfile, moves the declaration lox writes into vislox's file of the page (its wired
  neighbours as vislox's externs, pinned by UUID), adds the block to the lockfile and compiles. The block keeps its
  UUIDs; vislox checks that the config is exactly what it was, the block exactly as the base has it, and refuses
  otherwise. From then on the block is managed (λ): it is moved, renamed, given constants and settings, and wired
  like a block added here. *Adopt all*, next to *Arrange*, adopts every block of the page lox can rebuild and
  says why the rest stay Loxone Config's (lox's own reason, or a warning). Nothing is pending afterwards, since
  nothing the Miniserver runs changed: the adoption goes into git with the next Apply, and Undo (or *Hand back*)
  gives the block back to Loxone Config. Properties says why a block cannot be adopted (values or wires edited
  here, an extern of the module's own files), and *Hand back* why it is off. How:
  [editing.md](editing.md#adopting-blocks-72).

- **Switching times** (the same project and token, #73): Properties of a DayTimer has *Switching times* with *Edit
  switching times*: a day bar per operating mode, the seven weekdays first, then the timer's other modes (*Add a
  column for* holiday, vacation, day off, heating or cooling period, or one of the config's own modes; ✕ takes one
  out). Click an empty bar to add an hour there and drag to its end; drag an interval to move it, an end to resize
  it (snapped to 15 minutes; Alt: to the minute). Below, the selected mode's intervals take exact times
  (`24:00` ends the day), a value on an analog timer, *activate* (the output switches on only after a pulse on the
  timer's activation input) and *Delete*; *Copy to* puts them on the weekdays, the weekend or every day, and
  *Start from the times of* another timer fills the editor with its. *Save* writes a Lox Logic `schedule` set and
  the timer's claim into vislox's file of the module and compiles: the claim replaces the timer's whole schedule,
  so a weekday left empty is off all day. *Use the same times as …* lets several timers share one set; editing one
  of them asks whether the others change too. While live values come in, a Miniserver that runs other switching
  times (changed in the app) is named before you save: Apply would replace them. Saved and not deployed, the
  section says so, with *Revert*. A timer the module's own file claims is changed there. How:
  [editing.md](editing.md#switching-times-73).

- **Lighting moods** (#74): Properties of a lighting controller (LightController2) has *Moods*. *Circuits* lists the
  circuits in use (the named ones and those a mood other than "Viel Licht" sets; *Show all 18 circuits* for the
  rest), each with its actuator's name or *no actuator*, and its kind: switch, dimmer, colour (RGB) or tunable
  white, read off the moods. `lox logic lint`'s `no-actuator` finding shows on its circuit: a percentage on a
  circuit without an actuator only switches it. *Moods* lists them in the app's order with their values as Lox
  Logic writes them (`AQ1 on, AQ2 80%, AQ3 temp(60%, 2700K)`), the off mood ("Aus", always there), the built-in and
  motion moods, and *T5/n selects it* where a button on the controller's `Sel<n>` input selects the mood. Compared
  with another revision, changed values are marked and the Changes list has a line per mood changed, added or
  removed, and the new order. *Edit moods* (the same project and token) shows a row per mood and a column per
  circuit: rename, add, delete (not the built-in and motion moods) and reorder (↑ ↓) moods, and pick each circuit's
  value from those its kind takes (off, on, a percentage, `rgb`, white), with the numbers beside it. *Save*
  writes a Lox Logic `moods` set and the controller's claim into vislox's file of the module and compiles. A mood
  is known by its name: renaming one, or deleting one a button selects, is warned about first (the button then
  selects nothing, or a new mood that takes its scene id). *Use the same moods as …* lets several controllers
  share one set. Saved and not deployed, the section says so, with *Revert*. On a controller added or adopted
  here, each switch or dimmer circuit `AQ1` … `AQ5` can be declared the other kind. How:
  [editing.md](editing.md#lighting-moods-74).

- **Automatic Designer rules** (the same project and token, #87): *Edit rule* in a rule's *Rule* section opens its
  lines: *Active*, and each condition and action with its kind and fields (a time of day, days of the week, a block's
  state with a comparison, an operating mode, a weather field, a day of the month, day or night, sunrise and the
  like, a date; a block's command, switching a mode, a wait, a push notification, a mail, a message center entry). A
  block is picked with the Ctrl+K picker; a state or command offers those the config's rules already use on blocks
  of that type, and lox checks it against the app's capability table when it compiles (its refusal lists what the
  block offers). *Add condition*, *Add “or”* and *Add action*, ↑ ↓ and ✕. *Save* writes a Lox Logic `rule`
  declaration into vislox's file of the module and compiles; it shows as *not deployed* with *Revert*, and *Remove
  vislox's declaration* leaves the rule as last written. *New rule…* opens the same form for a named rule (its name,
  a condition and an action at least), which lox creates below the config's last rule; a rule created here is renamed
  in its form and taken out with *Delete rule*. A config without any rule cannot get one (the app decides where
  rules live), `when date` needs the Miniserver's time zone in the config, and rules the module's own files state
  are changed there. Nothing runs a rule or sends a notification, mail or message. How:
  [editing.md](editing.md#automatic-designer-rules-87).

- **Members of a central block** (#75): Properties of a CentralLight or CentralShade has *Members*: the lighting
  controllers or automatic blinds it drives, in the order the app lists them, each a link to its block (and its
  page). Compared with another revision, the members added are marked, those removed listed, and a new order
  shown; the Changes list says it by name (*Alle Lichter · member added: Diele*). *Edit members* (the same project
  and token) lists them with ↑ ↓ and ✕; *Add member…* (or Ctrl+K while the editor is open) picks one of the blocks
  the central block takes (LightController2 under a CentralLight, AutoJalousie under a CentralShade) that it does
  not list yet. *Save* writes a Lox Logic `.members` claim into vislox's file of the module and compiles; an empty
  list empties it. Saved and not deployed, the section says so, with *Revert*. A central block the module's own
  file claims is changed there. An alarm's or smoke alarm's members are shown, not edited: they stay with Loxone
  Config. How: [editing.md](editing.md#members-of-a-central-block-75).

- **Who may open an NFC Code Touch** (#76): Properties' *Access* (#68) has *Edit access* (the same project and
  token): a row per user or group, each with its access schedule, picked from the config's users, groups and access
  schedules by title (asked for with the admin token; a title two objects share cannot be picked, since Lox Logic
  names them by title), ✕ and *Add user or group*. *Save* writes a Lox Logic `access` set and the Code Touch's
  claim into vislox's file of the module and compiles. The set is the whole list: whoever it leaves out loses
  access, and an empty set revokes everyone. Before a save that takes access away, the editor names who loses it
  (*Ben loses access to Haustür*, *Save anyway*); the review and Apply's confirmation say it again. *Same rights as
  …* lets several Code Touches share one set, and *Change … too* rewrites it for all of them. Saved and not
  deployed, the section says so, with *Revert*. Users, groups, schedules, tags and codes stay with Loxone Config
  and the app; no tag or code is ever shown. How: [editing.md](editing.md#who-may-open-an-nfc-code-touch-76).

- **Push notifications** (#77): a notification's block on a page shows its *Message*: the notification it sends, the
  text sent when its input turns on and off (none: no message) and the minimum time between messages; the Changes
  list says a changed text or time as old → new. The notification lists the blocks that send it, and *Recipients*
  names those who get it as members of Full access (*Inherited*: no set takes it from them). *Edit recipients* adds
  or removes users and groups (no schedule) and writes a Lox Logic `recipients` set and the notification's claim;
  whoever it leaves out stops getting it, said before the save (*Anna stops getting Frostwarnung*, *Save anyway*)
  and again in Apply's confirmation. *Add block* → *Push notification* draws a notification Loxone Config made on
  the page with its message; wire its input from a block, and edit its message in Properties. The notification
  itself, and the message of a block Loxone Config drew, stay with Loxone Config. How:
  [editing.md](editing.md#push-notifications-77).

- **Touch Pure Flex tiles** (#78): select the device in the tree. *Tiles* lists the blocks linked to its API
  connector (the only blocks a tile's command reaches) and each tile by index: what its click and its arrows up and
  down do, the status text and the blocks each command addresses, marked against the compared revision; the Changes
  list says a changed function as old → new (`Flex Küche · tile 3 click: "…" → "…"`, `… click targets: Nord → Süd`).
  lox logic lint's `tile-broadcast` and `tile-dead-command` show on the tile. *Edit tiles* edits each function's
  command, text and targets (picked from the linked blocks) and writes a Lox Logic `tiles` set and the device's
  claim. The layout and the API link stay with Loxone Config. How: [editing.md](editing.md#tiles-of-a-touch-pure-flex-78).

- **Block settings** (#79): Properties has *Settings* on a block whose type keeps settings on its element, by their
  Lox Logic names and as Lox Logic writes them: a PulseAt's *At 18:58* and *Pulse sunset*, a memory flag's *Type*,
  a Daytimer's *On*/*Off*, the name the app shows (*Desc*), a meter's units, a program block's program (folded to
  its line count). Compared with another revision they are marked old → new, a program as a line diff; the Changes
  list says the same (`Abends · At: 18:58 → 06:58`, `Hue · program: + …`). A constant reads with its unit
  (`Time 30min`, not 1800). On a block the module manages through vislox (added or adopted) *Edit values* edits the
  label, the settings and the program (a text area; Ctrl+Enter saves), and a constant takes a unit (`1h`); the
  program goes into a file of its own next to the block's. On a block Loxone Config drew they are read-only: *Manage
  with Lox Logic* adopts it first. How: [editing.md](editing.md#block-settings-79).

For a `lox.toml` project:

```bash
vislox ~/loxone-config/project --admin-token-file ~/.config/vislox/admin-token
```

- `--admin-token-file`: a file holding a passphrase. Apply, Sync and Discard ask for it once per browser
  (`X-Vislox-Token`). Without it they are off; viewing needs no token.
- Apply commits with the change summary as the subject (`lox logic diff --summary`), then runs `git push` (after
  the personal-data scan, #71), which needs credentials for the project's remote. A verified push makes lox point
  `base` at its download in `backups/` (git-ignored); Apply copies that download over the project's base file and
  points lox.toml back at it, so the deployed config stays the project's base (and in git, when the project tracks
  it and it holds no personal data).
## Miniserver checks

- **Miniserver status and preflight**: a chip in the toolbar names the Miniserver lox talks to (serial and
  firmware) with a green, amber or red dot. Click it for the details: address, local or remote, TLS, whether a
  live-update slot is free, when lox's token expires, which Loxone Config version the deployed and pending
  configs were saved with, whether its web port and FTP (port 21) answer, and the route vislox takes (see
  [Route, TLS and Remote Connect](miniserver.md#route-tls-and-remote-connect)). Before Pull, Apply and Sync, and on
  start, vislox checks again, and refuses what could not work, with the reason: Pull, Apply and Sync when FTP is
  blocked (they copy the config over FTP, and live values only need the web port, so a firewall can block one and
  not the other), Apply also when the web port does not answer (lox restarts the program over it). The buttons
  stay on, since the cause may clear; their tooltip warns about what the last check found. A config saved with a
  different Loxone Config version than the firmware, an expired TLS certificate or token, a remote connection,
  no free live-update slot and a status reply that cannot be read are shown as warnings.
- **Verified TLS and Remote Connect**: with the Miniserver's serial known, vislox connects to a local Gen 2
  Miniserver under its CloudDNS name (`192-168-1-120.<serial>.dyndns.loxonecloud.com`) and checks its
  certificate, instead of trusting whatever answers at the IP address. When that is not possible it connects as
  lox does, and the chip warns that the certificate is not checked, and why. Away from home (lox's address does
  not answer), or with a lox context that names only a serial, it connects through Loxone's Remote Connect; the
  chip says *Remote Connect*, and Pull, Apply and Sync are refused there, with the reason.
- **The right Miniserver**: the preflight also checks that the Miniserver at lox's address has the serial the
  config belongs to, so Pull, Apply and Sync never talk to another device that DHCP gave the address to, or to
  the wrong context of a repository with several Miniservers. Another serial makes the preflight refuse all
  three and name both serials. When the address does not answer, or another Miniserver answers there, a
  discovery broadcast looks for the serial on the local network, and the reason names its new address and the
  `lox setup set --host …` command that points lox at it (with lox's scheme and port).

- **The Miniserver's config changed after the base**: the chip also asks the Miniserver when its config last
  changed (`jdev/sps/LoxAPPversion3`), and compares that with the date in the download the base came from. When
  the Miniserver's is later, someone saved from Loxone Config (or otherwise changed the config) after the base was
  taken: the chip turns amber, and a notice above Deploy and History says when, with **Pull from Miniserver** or
  **Sync from Miniserver**. Apply asks once more and warns in its confirmation. While the page is open it asks for
  the date every minute (the rest of the check every five minutes). An earlier date (an older config restored on
  the Miniserver) is shown in the chip's details, not as a warning.
- **Calendar entries changed in the Loxone app**: admins can add, change and delete operating-mode calendar
  entries in the app, and those edits live on the Miniserver only. vislox asks the Miniserver for the entries it
  uses and compares them with the deployed revision by UUID: a notice above Deploy and History says how many were
  *added*, *changed* (name, mode or date, read as *24 Dec → 26 Dec*) or *deleted* in the app, lists them (each a
  link to the entry), and offers **Pull from Miniserver** or **Sync from Miniserver**. The entry's Properties say
  *Changed in the app* or *Deleted in the app*, and the chip's details count them. Apply asks for them once more and
  warns in its confirmation. Without admin rights the Miniserver refuses, and nothing shows.
## Log, Debug-Monitor and operating inputs

- **Miniserver log** (bottom right): the Miniserver's system log, as `lox log` reads it, newest first. It is read
  when you open the panel, on Refresh, and after an Apply. The entries since the last program change (an Apply, or
  a save from Loxone Config) are marked, so you see what the new program logged as it started. A line that names a
  block (warnings carry the UUID of the block that raised them) gets a link that selects that block, and on the
  deployed revision that block gets a small `!` mark on the canvas.
- **Debug-Monitor** (`--monitor`, off by default): the Miniserver's live debug output (UDP 7777) in the same panel,
  marked *monitor*. **Start** asks the Miniserver to send it to vislox (`dev/sps/log/<address>`, as Loxone Config's
  Debug Information does; nothing in the config changes and the program does not restart), **Stop** ends it, and so
  do vislox exiting and a time limit (60 min). Only packets from the Miniserver's address count. Its CPU load and
  heap show in the Miniserver chip's details. See [debug-monitor.md](debug-monitor.md).
- **Operate virtual inputs** (opt-in: `--operate` with `--admin-token-file`): with live values on, a virtual
  input, or an input reference to one, gets controls in Properties: **Pulse**, **On** and **Off** for a digital
  input, a number field for an analog one. That lets you test a change the way Loxone Config's Liveview does.
  Each command needs a confirmation. The Miniserver's code shows next to the control, then vislox waits for the
  state update: the wire turns green or shows the new value, or the control says that no update came back.
  A block with a visualization password shows a lock. vislox asks for the password once per browser session and
  checks it (`checkuservisupwd`) before it sends the command; secured commands go one at a time, since each
  key is single-use. Every command sent (what, when, code) is listed under *Commands sent to inputs* in the History
  pane until vislox stops, and a request that failed before its command went out under *Not sent*, with the
  reason. vislox has one shared token and no user accounts, so the log names no one. Before each command the
  preflight checks that the Miniserver at lox's address reports the config's serial; when it reports another
  one, none, or cannot be checked, nothing is sent.

vislox itself never writes to the config: lox and git do, and only on Pull, Apply, Sync, Discard and when
values are edited (then only `lox logic compile` runs, into the pending config). Live values only
listen, apart from two read-only requests for the heating and cooling periods each time they connect, unless you
turn on **Query blocks on this page**, which sends read-only requests. A pull reads from the
Miniserver; only Apply writes to it, and with `--operate`, commands to virtual inputs.
## Mouse and keyboard

| Input | Click | Shift+click |
|---|---|---|
| History entry | show it | compare the viewed revision with this one (again: stop comparing) |

The *Viewing* and *compared with* menus in the toolbar pick any pair of revisions. Pick *nothing* to see
a revision without change marks. *Follow latest* jumps to each new revision as it appears. If history cannot be
loaded, vislox retries with capped backoff until it succeeds. It also retries a closed event stream after a delay;
each successful stream connection reloads history to catch up on changes missed while disconnected.

Keys:
- `Ctrl+K`: go to a page or block. `Ctrl+F`: filter the tree. `?`: list all shortcuts.
- `[` / `]`: previous / next change. PageUp / PageDown: switch pages.
- Arrow keys: pan. `+` / `-` / `0` / `1`: zoom in, zoom out, fit, 100 %.
- Esc: clear the selection.

With the mouse, drag to pan and scroll to zoom.

**Adopt into vislox** also covers objects in the project tree: user operating modes, writable calendar entries, virtual inputs and outputs, and virtual or Modbus devices with their I/O. Changes lists pending adoption with Revert; Apply names the objects in its review. Built-in modes and weekday-in-month entries stay read-only.
