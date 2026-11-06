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
- **Sidebar with the block's configuration**: the periphery and page tree (filter with Ctrl+F), and the
  selected block's properties: name, type, page, room, category, connections (each one a link),
  parameters and raw attributes.
- **Calendar entries as dates**: an operating-mode calendar entry shows *When* in words (*Every year on 24 Dec*,
  *Easter Monday (Easter +1)*, *4 Jul 2026 – 5 Sep 2026*, *Every year 1 Oct – 30 Apr*) and the mode it *Switches
  on*, linked to the `Mode` object. The tree's tooltip says the same, and the Changes list reads a changed entry
  as *24 Dec → 26 Dec* and *Holiday → Party* instead of raw attributes. An entry vislox cannot decode says why;
  its raw attributes stay under Advanced.
- **Operating modes** (Ctrl+K *Operating modes*, or **Year** on the tree's Calendar or Operating modes folder): a
  year calendar of when each mode is on. Each day carries the colour of every mode its entries switch on; ← / →
  go to other years (from 2009, Loxone's day 0), and a mode's chip hides or shows its days. Hovering a day lists its entries, and clicking
  a day or an entry in the list beside the year selects the entry (Properties). Today is outlined. With live
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

- **History** (right): every revision, newest first. The config on the Miniserver is marked *on Miniserver*.
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
- **Navigation**: Ctrl+K finds any page or block. There is a minimap, a "Changed pages only" filter, and links
  (`#page=…&sel=…&rev=…`) that reopen the same page, block and revision. The browser's Back button works.
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
  off). Values show on the current, deployed revision only.

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
  review it like any other change. **Apply** deploys it with `lox config push --force` (download, drift check,
  upload, program restart, verify), then commits the project and pushes it to its git remote. Each step shows
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

- **Edit values** (for a `lox.toml` project, with the admin token): Properties' *Edit values* changes a block's
  name, its room and category, the constant of an unwired input (lox checks it against the documented range) and the settings of a
  physical input or output or a device. *Save* writes them into vislox's own file of the module and runs
  `lox logic compile`, so they show as the pending revision; nothing is deployed until Apply. Each value saved and
  not deployed says *not deployed*, with *Revert*; Discard drops them all. Blocks the module manages are edited in
  their own `.loxl`. How it works, and what comes next: [editing.md](editing.md).

For a `lox.toml` project:

```bash
vislox ~/loxone-config/project --admin-token-file ~/.config/vislox/admin-token
```

- `--admin-token-file`: a file holding a passphrase. Apply, Sync and Discard ask for it once per browser
  (`X-Vislox-Token`). Without it they are off; viewing needs no token.
- Apply commits with the change summary as the subject (`lox logic diff --summary`), then runs `git push`, which
  needs credentials for the project's remote. A verified push makes lox point `base` at its download in
  `backups/` (git-ignored); Apply copies that download over the project's base file and points lox.toml back at
  it, so the deployed config stays in git.
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
a revision without change marks. *Follow latest* jumps to each new revision as it appears.

Keys:
- `Ctrl+K`: go to a page or block. `Ctrl+F`: filter the tree. `?`: list all shortcuts.
- `[` / `]`: previous / next change. PageUp / PageDown: switch pages.
- Arrow keys: pan. `+` / `-` / `0` / `1`: zoom in, zoom out, fit, 100 %.
- Esc: clear the selection.

With the mouse, drag to pan and scroll to zoom.
