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
- **History** (right): every revision, newest first:
  - *Pending*: the compiled `out.Loxone` of a `lox.toml` project, compared with the deployed `base`.
    This is what `lox config push` would apply.
  - *Changed on disk*: snapshots taken while vislox runs, whenever the file changes.
  - *Git commits*: the file's history, e.g. from `lox config pull`.
  - **Pull from Miniserver** (in a `lox config init` repository): runs `lox config pull`, which downloads the
    Miniserver's current config and commits it when it changed. The new backup appears at once.
  - *Miniserver backups*: the Miniserver keeps a backup of each config it loads (`/prog/sps_<ver>_<stamp>.zip`;
    one Miniserver kept its last 20). **Read** lists them and downloads each new one once (FTP, read-only). A backup with the
    bytes of the Pull next to it shows as that Pull (a *Miniserver backup* pill); the others are listed under the Pulls they
    were saved between (*Between Backup v273 and Backup v274*, *Since the last Pull*), each compared with the
    save before it. So every save from Loxone Config can be reviewed on its own, even when nobody pulled in
    between, and shift-click compares any two.
- **Change review**: added (green), changed (amber), removed (red, dashed ghost) and moved (blue, ghost at
  the old place) blocks. Wires and parameters are marked too: changed values show old → new in the
  properties. The Changes list can be filtered by kind, and clicking an entry jumps to it.
- **Readable at every zoom**: zoomed out, each block shows only its name, at a readable size. From about 80 %
  the full detail appears. Gates show their operation (`AND`, `Less`, `Divide`) instead of Loxone's automatic names
  (`O208698`); a named gate keeps its operation in grey, as Loxone Config 16 shows it. Selecting a block highlights its wires and neighbours and dims the rest.
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

- **Apply, Sync, Discard** (for a `lox.toml` project, when started with an admin token): edit the project's Lox
  Logic module and compile it with `lox logic`, by hand or with an agent of your own working in the repository
  (see [docs/agent-workflow.md](docs/agent-workflow.md)). The compile shows as the pending revision, so you
  review it like any other change. **Apply** deploys it with `lox config push --force` (download, drift check,
  upload, program restart, verify), then commits the project and pushes it to its git remote. Each step shows
  as it happens, with how long it took. vislox follows the program restart itself: the Miniserver going out of
  service, back online after N s, and the new config loaded (its structure version changed). When Apply
  finishes, the view switches to the deployed revision, compared with the one it replaced, and turns live
  values on. **Changed by this Apply** lists the changed blocks with their first live values. **Discard** puts
  the module back to the last commit. **Sync from Miniserver** makes what the Miniserver runs now the project's
  base (`lox config merge`: changes saved from Loxone Config, rules edited in the app), then commits and pushes;
  it waits until nothing is pending. When merge refuses because the Miniserver holds custom changes no download
  carries, Sync makes a plain download the base instead (rules edited in the app are then not taken over).

- **Miniserver status and preflight**: a chip in the toolbar names the Miniserver lox talks to (serial and
  firmware) with a green, amber or red dot. Click it for the details: address, local or remote, TLS, whether a
  live-update slot is free, when lox's token expires, which Loxone Config version the deployed and pending
  configs were saved with, and whether its web port and FTP (port 21) answer. Before Pull, Apply and Sync, and on
  start, vislox checks again, and refuses what could not work, with the reason: Pull, Apply and Sync when FTP is
  blocked (they copy the config over FTP, and live values only need the web port, so a firewall can block one and
  not the other), Apply also when the web port does not answer (lox restarts the program over it). The buttons
  stay on, since the cause may clear; their tooltip warns about what the last check found. A config saved with a
  different Loxone Config version than the firmware, an expired TLS certificate or token, a remote connection,
  no free live-update slot and a status reply that cannot be read are shown as warnings.
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
- **Miniserver log** (bottom right): the Miniserver's system log, as `lox log` reads it, newest first. It is read
  when you open the panel, on Refresh, and after an Apply. The entries since the last program change (an Apply, or
  a save from Loxone Config) are marked, so you see what the new program logged as it started. A line that names a
  block (warnings carry the UUID of the block that raised them) gets a link that selects that block, and on the
  deployed revision that block gets a small `!` mark on the canvas.
- **Debug-Monitor** (`--monitor`, off by default): the Miniserver's live debug output (UDP 7777) in the same panel,
  marked *monitor*. **Start** asks the Miniserver to send it to vislox (`dev/sps/log/<address>`, as Loxone Config's
  Debug Information does; nothing in the config changes and the program does not restart), **Stop** ends it, and so
  do vislox exiting and a time limit (60 min). Only packets from the Miniserver's address count. Its CPU load and
  heap show in the Miniserver chip's details. See [docs/debug-monitor.md](docs/debug-monitor.md).
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

vislox itself never writes to the config: lox and git do, and only on Pull, Apply and Discard. Live values only
listen, unless you turn on **Query blocks on this page**, which sends read-only requests. A pull reads from the
Miniserver; only Apply writes to it, and with `--operate`, commands to virtual inputs.

## Run

```bash
cargo install --path .          # or: cargo run --release -- <path>

vislox ~/loxone-config          # a `lox config init` repository (<serial>/config.Loxone)
vislox ./my-project             # a lox.toml project: base vs compiled out.Loxone
vislox config.Loxone            # one .Loxone file, or a backup .zip
vislox ~/loxone-config --serial 504F94AABBCC   # several Miniservers in one repository
vislox --discover               # list the Miniservers on the local network
```

Started without a path or `--ctx` in a directory with no config (and without `--no-live`), vislox lists the
Miniservers on the local network and how to set one up with lox.

Or with Docker, which bundles git and lox (see [docker/README.md](docker/README.md)):

```bash
docker run -d -p 7878:7878 -v ~/loxone-config:/data/repo ghcr.io/hakubo/vislox:main
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

### Apply, Sync and Discard

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

### In the app

The structure file (`LoxAPP3.json`) is read, read-only and with lox's user, when live values start (each live
session reads it anyway), and when a user clicks *Read from the Miniserver* in Properties (`POST /api/app/load`,
which needs an `X-Vislox` header). vislox never reads it on its own. The last one is kept per Miniserver (serial,
else address) in `VISLOX_CACHE`, else `$XDG_CACHE_HOME/vislox`, else `~/.cache/vislox`, so the next start has it;
Properties says when it was read. Reading the same file again only counts the read, so browsers re-render only
when it changed. Icons are downloaded by the name the structure file gives
(`IconsFilled/lightbulb.svg`, or an older UUID, fetched as `<uuid>.svg`; SVG comes as text, often gzipped, PNG as
binary) and cached next to it once they arrived whole: a name keeps its icon, and one name is downloaded once at a
time. `GET /api/app/icon?name=…` serves only names the structure
file lists, so it cannot be used to read anything else from the Miniserver. With `--no-live` nothing is read.

The structure file belongs to the config the Miniserver runs: mismatches are shown only on that revision (the
current one, not *Pending*). The config side (shown, room, category) comes from each block's `<IoData>`, so a
pending change already shows how it will look.

Not built yet: an *App* view that lists the controls by room the way the app does.

### Schedules

The Miniserver sends each Daytimer's schedule as an event table (its `entriesAndDefaultValue` state, for the
blocks the app shows): the default value, and per entry the operating mode by its number, from, to,
need-activate and the value. vislox reads the config's schedule in the same terms (`src/schedule.rs`: the
`<Entry>` rows of a `DayTimer`, or of a block's `<Timer>`, whose live UUID is its `UUIDTimer=`) and compares them
per mode. A digital Daytimer's values do not count: the Miniserver sends 0 where the file has 1.

An edit in the app is a *custom change* on the Miniserver (`/sys/custom`) until Loxone Config saves the program; a
download does not carry it. `lox config push` refuses while one is pending, and with `--discard-custom-changes`
(Apply asks first) the upload replaces it with the config's switching times. So a schedule marked *changed in the
app* is lost on the next Apply unless it is taken into the config (in Loxone Config: load from the Miniserver,
save). Without live values nothing is compared, and Apply's confirmation says so when the pending config has
schedules.

Weather comes from the weather server's states (`weatherServer` in the structure file, only with a weather
subscription): its current weather, else the forecast hour that has begun. A `WeatherData` block's `WType=`
picks the field (1 temperature, 2 dew point, 3 humidity, 4 wind speed, 5 wind direction, 7 solar radiation,
9 precipitation, 10 weather type, 11 air pressure, 26 perceived temperature); such values are marked computed.
A Status block's icon is the one its `iconAndColor` state names, shown only when the structure file lists that
icon: `/api/app/icon` fetches no name a live state (which a user can set in the app) brings in. It fetches only
names of an icon's shape, `<folder>/<file>.svg` or `.png` (never under `jdev`, `dev`, `data`, `sys`, `prog`, …) or
an icon UUID (as `<uuid>.svg`), so no name can be a command. Icons the Miniserver sends gzipped without the gzip
trailer are kept unpacked.

### Miniserver status

The status chip and the preflight only read, and never log in: a TCP connect to the web port in lox's `host`
(443 by default) and to FTP (21), then FTP's greeting, and `jdev/cfg/apiKey`, which the Miniserver answers
without a login, and only informs (serial, firmware, TLS, local, slots): the request follows no redirect to
another host, as lox's own client. The token's expiry comes from lox's `token.json`. vislox uses lox's token on purpose and
requests none of its own, so it does not show up as a client of its own in the Miniserver's token list (lox's
is named `lox-cli`). A check is reused for up to a minute for the chip (`GET /api/miniserver`; `?fresh=1` checks again unless a
check started in the last 2 s), and for 10 s before Pull, Apply and Sync, so that retrying an Apply does not
check each time.
With `--no-live` nothing is checked.

The serial the config belongs to is, in this order: the repository's `<serial>` folder, `serial` in lox.toml, the
lox context's `serial` (`lox setup set --serial`). Without one, nothing is compared (the chip says which
Miniserver answers). The serial at the address comes from `jdev/cfg/apiKey`, which also works across VLANs and
routers; a reply that cannot be read does not count as another serial. Only when there is a serial to look for,
and the address does not answer or answers with another serial, does vislox send Loxone's UDP discovery
broadcast (one `0x00` byte to port 7070; Miniservers answer to port 7071 with name, address, serial, firmware
and the date of the loaded program). A search is reused for a minute. Port 7071 is shared with other listeners
(`SO_REUSEADDR`); when it still cannot be bound, the chip and `--discover` say so, since replies are then
missed. Broadcasts stay in the local network: they do not cross VLANs or routers, Remote Connect, or Docker's
default bridge network.

The config date is the one request that needs lox's user and password (HTTP basic auth, as Apply's restart
probe; no token is requested). The Miniserver sends `lastModified` of its `LoxAPP3.json`, in its local time.
The base's own date comes from the download lox took it from: `backup.zip` next to a repository's `config.Loxone`
(`lox config pull`), or, in a project, the zip in `backups/` beside the newest `.Loxone` that equals the base byte
for byte (`lox config push`, `merge`, Sync). When no download matches (a base edited by hand), nothing is
compared, and neither is a backup `.zip` opened on its own (a look at an old config). The date is coarse: a push
that changes only logic can leave it as it was, so equal dates do not prove equal configs, but a later date on the
Miniserver does mean the base is old. When the Miniserver does not answer the request (no rights, wrong password,
a timeout of 5 s), the chip's details say that it cannot tell; that is not a warning. The date is asked for apart
from the rest of the check, so the preflight before Pull, Apply and Sync never waits for it.

Not shown yet: the token's rights (Config, FTP, program restart) and whether the password is weak. The
Miniserver reports them only at login (`tokenRights`, `unsecurePass`), and lox does not keep them.

### Calendar entries changed in the app

The Miniserver lists the calendar entries it uses with `jdev/sps/calendargetentries` (lox's user, HTTP basic
auth; Loxone's Operating Mode Schedule datasheet, "Administrative Commands"). vislox only reads: it never sends
`calendarcreateentry`, `calendarupdateentry` or `calendardeleteentry`. It asks when a structure file was read
successfully (each live session reads one, and so does *Read from the Miniserver*), at most once a minute; after Pull, Apply and
Sync; and before Apply (`GET /api/calendar?fresh=1`, which needs an `X-Vislox` header, reuses a good read from the
last 2 s; a failed one is never reused). One request is in flight at a time, also after vislox stopped waiting
for it (8 s). It never asks on a timer. `GET /api/calendar` without `fresh` only reports the last read. With
`--no-live` nothing is read.

The command needs admin rights. When the Miniserver refuses (a code other than 200, HTTP 401 or 403), the chip's
details say *can't tell* with the reason, and the notice never shows. When the check before Apply fails otherwise
(no answer, the Miniserver restarting, vislox not answering), Apply's confirmation says it could not check.

The answer numbers its dates differently from the `.Loxone` file (`calMode` 0 a day every year, 1 Easter, 2 one
date, 3 one period, 4 a range every year, 5 a weekday of the month; the file's `Mode=` is absent, 1, 3 or 4), so
entries are compared by their decoded dates, never by raw numbers: one date is a period of one day, and
`operatingMode` is the file's `CalType=`. A date vislox cannot read on either side (`calMode` 5, which the file
format does not document) is not compared; its name and mode still are. The comparison is with the deployed
config: the newest commit of the watched file (the last Pull, Apply or Sync), not an uncommitted edit on disk or a
pending compile; without git, the file on disk.

Not verified yet: whether `lox config merge` (Sync) takes over entries edited in the app, or refuses because the
base and the download differ in more than rules (Sync then makes the plain download the base). The notice offers
both Pull and Sync, as the config-date notice does.

### Operate virtual inputs

```bash
vislox ~/loxone-config --admin-token-file ~/.config/vislox/admin-token --operate
```

This is the only part of vislox that sends commands to the Miniserver, so it needs both flags, and live values
from the Miniserver itself (not `--no-live` or `--live-from`). Commands go over HTTP as the lox user
(`jdev/sps/io/{uuid}/{cmd}`, or `jdev/sps/ios/{hash}/{uuid}/{cmd}` for a block with a visualization password).
vislox takes commands only for `VirtualIn` objects of the deployed config. A digital input takes `pulse`, `on`
or `off`. An analog input takes a number, within its `MinVal`/`MaxVal` when it checks values. Physical
inputs, outputs and every other block are refused. The visualization password stays in the browser's session
storage and is never logged. The command log is kept in memory.

The Miniserver connection is opened when a browser turns live values on, and closed shortly after the last
one turns them off. With **Query blocks on this page**, the browser asks for the page's queries with
`POST /api/live/query` (`{rev, page}`, only the current, deployed revision; it needs the `X-Vislox` header, so
that no other web page can make vislox send requests), and holds `GET /api/live/query?token=…` open with the
one-use token it gets while it shows the page. vislox sends `jdev/sps/io/<uuid>/all` for each block the page
needs on that same WebSocket, at most 20 a second without waiting for earlier answers, each block at most every
2 s. Blocks whose wanted outputs the Miniserver streams are left out, and so is a block that answers with an
error code (until the next connection) or does not answer three times in a row (for a minute). The values reach
every browser with the others on `/api/live`; the page's own stream says how its blocks did (`query` events). How an answer maps to connectors is in `src/query.rs`:
outputs come in config order (`nr`), under labels that are not the connector keys (`R` and `E` for a Formula's
`AQ` and `TQ`), so they map by position when the counts match, else by a label that names the key (a meter
leaves out its weekly totals), else by a table for the types that leave other outputs out (HVAC controller,
presence detector, roof window, music player, …). Blocks with one value (memory flag, mode, actuator, system
variable) give it to their first output. A lighting controller, a State block and a smoke alarm are not
queried: their answers do not say which connector a value belongs to. During Apply, vislox also asks the Miniserver every 0.5 s, read-only, whether it answers
(`jdev/cfg/apiKey`, no login) and which structure it runs (`jdev/sps/LoxAPPversion3`). It does this only from
the program restart until the new config runs, and only if the Miniserver answered before the upload.
`--no-live` turns this off as well.

When the Miniserver refuses or drops the connection, the live status names the reason it gives (its close or
response code) and says what to do. How long vislox waits before it tries again depends on the reason:

| Code | Live status | vislox |
|---|---|---|
| 4003 | blocked after too many failed logins | stops: check the user and password (each attempt makes the block longer) |
| 401, 403, 4006, 423 | login refused, no rights, user disabled | stops: fix the user, then turn live values off and on (a 401 or 403 of the structure file's HTTP request is first retried twice) |
| 4004, 4005 | a user was changed | logs in again after 1 s |
| 4007 | firmware update | tries again after 30 s, up to every 2 min |
| 503, out of service | restarting, going offline | tries again after 5–10 s, up to every 30–60 s |
| 4008, 901 | no free slot (31 clients), too many connections | tries again after 1 min, up to every 5 min |
| other | offline | tries again after 1 s, doubling up to 30 s |

Once stopped, vislox does not log in again on its own: not for a reload, another tab or a reconnecting
browser, only when a user turns live values off and on (`POST /api/live/retry`).

A Pull, Apply or Sync that fails with one of these codes shows the same explanation above lox's message. Only
the Miniserver's own wording counts (`Code 401`, `HTTP 503: /jdev/…`, `close code 4003`); git's `HTTP 403` does not.

The **Miniserver log** runs `lox log -n 300` with the same lox configuration and context as live values
(`GET /api/log`, which needs an `X-Vislox` header, since lox logs in for it). It only reads. One `lox log` runs
at a time, and requests that come in while it runs share the next one (its failure too), so none gets a log read
before it asked. When lox fails, the panel says why and keeps the entries it had. A program change is a
`Program changed by user` line, with the `PRG got restart command` logged up to 2 s before it; a reboot or a plain
program restart is not one. After an Apply, the log read before it marks no blocks until it is read again.
`--no-live` turns it off.

The **Debug-Monitor** listens on UDP (`--monitor`, `--monitor-port 7777`, `--monitor-ip <this host>`, or the
`VISLOX_MONITOR*` environment variables) and streams to the browser on `GET /api/monitor`. Start and Stop are
`POST /api/monitor/start` and `/stop` (with the `X-Vislox` header). `GET /?<serial>` answers a Monitor Server request
with the XML that points the Miniserver at vislox. To turn it off: Stop, then start vislox without `--monitor`.

**Miniserver backups** are read only when a user clicks *Read* in the History (`POST /api/backups/read`, with
the `X-Vislox` header and the same preflight as Pull), and after a Pull once they were read. lox's
`ftp::list_backups` lists `/prog`, and each backup vislox has not seen is downloaded with `ftp::download_backup`,
one at a time with a pause between them, and decompressed with `archive::extract_and_decompress` (which checks
the LoxCC size and CRC-32). Its program is the same bytes `lox config pull` commits, so its git blob id finds
the Pull that took it. The program is cached by that blob id in `VISLOX_CACHE` (as the structure file), under
`<serial or address>/backups/`, with `index.json` (file name to blob id) and the last listing (`list.json`). A
backup is never downloaded twice, and one the Miniserver has deleted stays in the History (*no longer on the
Miniserver*). The cache shows with `--no-live` too, and links to a backup keep working; reading needs a cache
directory, and is refused without one. A backup that cannot be downloaded or read (one still being written) is
listed under *Could not be read* with the reason, and tried again on the next Read; the others are read anyway.
One FTP session at a time: a listing (30 s) or download (120 s) that takes too long stops the read, and the next
one waits until that session has ended.

Saves in a row with the same program are one. A backup shows as a Pull only when it is next to it in time (the
Pull took it, or nothing changed in between); a save that goes back to an older program shows in its gap, *same
as* that revision. The time in a file name is the Miniserver's local time, in the zone its config names
(`Timezone="Europe/Warsaw"` in the `Document`, from the newest backup), never in the zone of the computer vislox
runs on (in Docker that is UTC). A Pull is placed by the date in its subject (`Config backup 2026-11-05 23:55:04`,
the backup it took), else by its commit time in that zone. With no zone known, stamps count as UTC, and a Pull
without a date is placed by the backup with its bytes. `--no-live` turns reading off. vislox does not write
backups to git: a Pull commits the newest one.

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
- `src/schedule.rs` reads a Daytimer's (or a room controller's) switching times from the `.Loxone` file, in the
  terms of the Miniserver's Daytimer event table, for Properties, the change list and the live comparison.
- `src/calendar.rs` decodes a calendar entry's date from the `.Loxone` file's attributes (as lox's
  `loxone-format.md` documents them: `Mode=` the kind, `From`/`To` days since 2009-01-01, `CalType=` the mode's
  number) and words it. The view carries the decoded date by kind (`calendar.date`), and the config's modes by
  number (`modes`), for `web/modes.js`, the Operating modes view, which lays the dates out on a year. It
  also decodes the entries of `calendargetentries` and compares them with a config's (`calendar::drift`).
- `src/appcalendar.rs` reads the Miniserver's calendar entries (`calendargetentries`) when live values start,
  after Pull, Apply and Sync and before Apply, for the *changed in the app* notice (`GET /api/calendar`).
- `src/actions.rs` runs Apply (`lox config push --force`, `git commit`, `git push`), Sync (`lox config merge`,
  `git commit`, `git push`) and Discard (`git checkout` of the module, lock and out). Apply's steps follow lox's
  progress lines as they come.
- `src/restart.rs` follows the program restart during Apply: out of service, back online, new config loaded.
- `src/backups.rs` lists and downloads the Miniserver's own backups for the History and caches their programs
  by blob id; `history.rs` shows each with the Pull of the same bytes, or between the Pulls around it.
- `src/pull.rs` runs `lox config pull` for the Pull button (`POST /api/pull`, which needs an `X-Vislox` header so
  that no other web page can trigger it), then refreshes the history.
- `src/monitor.rs` is the Debug-Monitor: the UDP listener, the address filter, Start/Stop and the Monitor Server
  answer. `src/monframe.rs` parses its frames (checksum, header, parameters, format strings, system stats).
- `src/log.rs` runs `lox log` for the Log panel (`GET /api/log`), splits it into entries, marks the last program
  change and picks out the UUIDs a line names.
- `src/miniserver.rs` checks the Miniserver for the status chip and the preflight before Pull, Apply and Sync
  (`GET /api/miniserver`), including that the serial at the address is the config's, and when its config last
  changed; `src/configdate.rs` finds the same date for the base, in the download it came from.
- `src/structure.rs` keeps the Miniserver's structure file (`LoxAPP3.json`) for the In the app section
  (`GET /api/app`), caches it and its icons on disk (`GET /api/app/icon`), and reads it again on request.
- `src/discover.rs` is Loxone's UDP discovery (`vislox --discover`, and finding a Miniserver whose address
  changed).
- `src/operate.rs` sends the commands of `--operate` to virtual inputs (`POST /api/operate`, with the admin
  token) and keeps the command log.
- `src/live.rs` streams the Miniserver's states with lox's `loxone-client` crate. A wire's source
  connector has the UUID of the state it carries, so the browser matches values to wires by UUID. Values
  reach the browser on `/api/live` (server-sent events). `src/query.rs` maps a queried block's answer to its
  connectors, and `model::query_plan` picks the blocks a page needs.
- `src/session.rs` is one WebSocket session (connect, log in, subscribe, stream, and the opt-in block queries),
  built from `loxone-client`'s parts so that it keeps the Miniserver's close code. `src/reason.rs` turns close and response codes into a
  reason, a plain sentence and a backoff.
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
VISLOX_BIN=target/release/vislox node tests/ui/apply.mjs   # Apply's steps, with a stand-in lox
```

CI (`.github/workflows/ci.yml`) runs all of these on every push to `main` and on every pull request.
