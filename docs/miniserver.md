# What vislox reads from and sends to the Miniserver

Reference for every connection vislox makes, what it caches, and how it reacts to errors. `--no-live` turns off
vislox's own connections; Pull, Apply and Sync still run lox. The short version is in the README's [Safety](../README.md#safety) section.

## Route, TLS and Remote Connect

vislox picks its route to the Miniserver in the background after start (`--route auto`, the default; or
`VISLOX_ROUTE`); until then, and whenever picking fails or takes over 30 s, it uses lox's host:

| Route | When | Certificate |
|---|---|---|
| CloudDNS name | lox's host is `https://` and a LAN IPv4 address that answers, and the serial is known | checked |
| lox's host | the CloudDNS name does not work (see below), there is no serial, or lox's host is a name or `http://` | not checked (`verify_ssl` in lox's context: checked) |
| Remote Connect | lox's context names only a serial, lox's address does not answer, or `--route remote` | checked |

The serial is the config's (repository folder, lox.toml), else the lox context's (`lox setup set --serial`).

- **CloudDNS name**: a Gen 2 Miniserver with the CloudDNS certificate (Loxone gets it from Let's Encrypt for
  `*.<serial>.dyndns.loxonecloud.com`) answers locally under `<ip-with-dashes>.<serial>.dyndns.loxonecloud.com`,
  which Loxone's DNS resolves to the local address. vislox checks that the name resolves to lox's address and
  that the certificate is valid for it (with the public web roots, as a browser does), with `jdev/cfg/apiKey`,
  which needs no login. A router with DNS rebinding protection may refuse such names (allow
  `dyndns.loxonecloud.com`); a custom or expired certificate, or the certificate of another serial, does not
  check out. Then vislox uses lox's host and the chip's warning says which of these it was. A certificate for
  another serial means another Miniserver may have the address.
- **Remote Connect** (firmware 17.0 and later): vislox asks `connect.loxonecloud.com/getip?snr=<serial>` for the
  relay URL and checks its certificate. The Connect Service's answers become plain messages: `404` no relay
  (Remote Connect off, or firmware older than 17.0; the old `dns.loxonecloud.com` service is not used), `504`
  the Miniserver is not connected to a relay, `409` an unsecure password blocks external access, and so on.
  When the relay does not answer, outbound TCP 20000–49000 to `*.remcov3.loxonecloud.com` is probably blocked.
  The relay carries only HTTPS and WSS: FTP does not go through it, so Pull, Apply and Sync are refused on
  this route. Live values, the structure file and the status chip work.
- `--route local` never uses Remote Connect; `--route remote` always does. A loopback host (`127.0.0.1`) is
  always taken as it is, and `--live-from` takes lox's host without picking.
- **Picked again**: every minute while lox's address does not answer (a Miniserver still booting after a power
  cut, a laptop back home: vislox goes back to the local route when it answers), and after a failed connection
  (at most every 30 s). A local route that worked moves to Remote Connect only when lox's address stays silent
  for a minute, so a program restart does not. Once the CloudDNS name was verified, vislox never falls back to
  an unchecked connection to that address: it stays on the name, and the chip says why it fails (restart vislox
  to accept the fallback). Live values reconnect on the new route.
- **The right Miniserver**: the chip reads who answers at lox's host (where Pull, Apply and Sync go) without
  checking the certificate (`jdev/cfg/apiKey` is public, and no password is sent), so another device at the
  address still shows its serial and is refused. On a Remote Connect route picked because lox's address did not
  answer, vislox still looks for the Miniserver on the LAN and names its new address with the `lox setup set`
  command.

The route is for vislox's own connections: live values, the status chip and preflight, the structure file, the
app's calendar entries, Apply's restart probe and operating inputs. On a verified route all of them check the
certificate, and no request that carries lox's user or password goes out unchecked (see [architecture.md](architecture.md)). lox's
commands (Pull, Apply, Sync, `lox log`, the Debug-Monitor's Start and Stop) use lox's host, and lox does not
check certificates yet.

## In the app

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

## Schedules

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

## Miniserver status

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

## Calendar entries changed in the app

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

## Hardware changes

An object is a device by its type: the extensions (`LoxAin`, `LoxDigin`, `LoxREL`, `LoxTree`, `LoxAIR`, …), the
bus interfaces (`Comm485`, …), Tree devices (`TreeDevice`, the kind from `TreeType=`) and Air devices
(`LoxAIRDevice`). A `Serial=` alone does not make one: the Miniserver, client Miniservers, Intercoms and network
devices carry a MAC there. Its inputs and outputs belong to it. What counts as reconfiguring it
(`src/hardware.rs`):

- adding or removing the device;
- its identity (`Serial=`, an Air device's `IP=`): *replaced* from one real serial to another, *paired* from none
  (or `00000000`, a device added offline) to one, *unpaired* the other way;
- on the device or its inputs and outputs, only the attributes matched to its configuration block (the table
  below). Anything else does not count: names, mounting, the `V=` a newer Loxone Config writes on every object,
  and attributes not matched yet.

The device types and the configuration blocks come from
[Inside-The-Loxone-Miniserver](https://github.com/sarnau/Inside-The-Loxone-Miniserver) (`NAT/`), bus captures
of 2019–2021 firmware. The `.Loxone` attribute names were matched to them by hand, for the devices in the
reference config so far:

| Device | Attribute | Configuration field |
|---|---|---|
| AI Extension (`LoxAin`), its `VoltageIn` | `MinChange` (V), `MinTime` (ms), `AverageTime` (s) | threshold (mV), minimal time space (ms), mean value calculation time (s) |
| Touch Tree `0x8003` (and the other Touches) | `AC` | audible acknowledgement |
| Room Comfort Sensor Tree `0x8010` | `SSC` (s; lox's examples save `900`) | send interval for temperature and humidity (minutes, default 15) |
| every device | `Serial`, `TreeType`/`SubType`, `HwVersion` | the device's identity and type |

Not matched yet: the DI Extension's frequency inputs (a bitmask in its block; no saved config with one set), the
Tree sensors' own `MinChange`/`MinTime` (they may be the Miniserver's filter, not the device's), and the other
devices of the NAT folder (AO, Touch Pure, Valve Actuator, Damper, LED, RGBW 24V Dimmer, Alarm Siren, Leaf,
Weather Station, Motion Sensor). The NAT table's `0x801A` is named *Presence Sensor Tree*, as lox's
`tree_kinds` and Loxone Config saves have it.

The devices on the bus come from `lox extensions -o json` (name, serial, online), run like `lox log` with
lox's configuration and context (`GET /api/hardware`, which needs an `X-Vislox` header since lox logs in; one read
at a time). It only reads. Apply's confirmation lists the devices from `GET /api/apply/changes?rev=&base=` (`devices`)
(the pending compile compared with what is deployed, as the list after Apply is), and reads the bus only when
the change adds, pairs or replaces a device. It waits at most 6 s for both, then asks anyway and says the bus
was not checked. After Apply, the bus is read every 15 s for 3 minutes, only when the Apply reconfigured a
device. Devices match by serial. An Air device (its `IP=` is not the serial the bus lists) and a device not
paired yet match by name, only to show their state: they are never reported missing. lox 0.14
has no `lox hw status`; when it does, it compares the config's serials with the Miniserver's `/data/status`
itself. Not shown yet: the reset reason a device reports when it starts (power-on, watchdog, …), which would
say why one went offline around an Apply; lox does not expose it. `--no-live` turns the bus reads off.

## Operate virtual inputs

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

## Live values connection

The Miniserver connection is opened when a browser turns live values on, and closed shortly after the last
one turns them off. Each time it opens, vislox asks for the heating and cooling periods on it
(`jdev/sps/calendargetheatperiod`, `jdev/sps/calendargetcoolperiod`, both at once, one 10 s deadline), again
after 10 s, 30 s and every 5 minutes for one not read yet, and sends them to the browsers with the live values
(`periods` on `/api/live`; a value of an earlier connection has `stale`, and `failed` when this one's read failed). With **Query blocks on this page**, the browser asks for the page's queries with
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

## Miniserver log

The **Miniserver log** runs `lox log -n 300` with the same lox configuration and context as live values
(`GET /api/log`, which needs an `X-Vislox` header, since lox logs in for it). It only reads. One `lox log` runs
at a time, and requests that come in while it runs share the next one (its failure too), so none gets a log read
before it asked. When lox fails, the panel says why and keeps the entries it had. A program change is a
`Program changed by user` line, with the `PRG got restart command` logged up to 2 s before it; a reboot or a plain
program restart is not one. After an Apply, the log read before it marks no blocks until it is read again.
`--no-live` turns it off.

## Debug-Monitor

The **Debug-Monitor** listens on UDP (`--monitor`, `--monitor-port 7777`, `--monitor-ip <this host>`, or the
`VISLOX_MONITOR*` environment variables) and streams to the browser on `GET /api/monitor`. Start and Stop are
`POST /api/monitor/start` and `/stop` (with the `X-Vislox` header). `GET /?<serial>` answers a Monitor Server request
with the XML that points the Miniserver at vislox. To turn it off: Stop, then start vislox without `--monitor`. Details: [debug-monitor.md](debug-monitor.md).

## Miniserver backups

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

## Which config the Miniserver runs

`GET /api/running` (`?fresh=true` skips the cache) asks the Miniserver, over vislox's route and with its TLS
check, for the `/prog` listing (`/dev/fslist//prog`), its system log (`/dev/fsget/log/def.log`, as `lox log`;
it needs an administrator) and its firmware (`/dev/cfg/version`). All three only read. The answer is kept for a
minute, one check runs at a time, and a check that takes over 30 s fails. A failed check is still a 200, with
`error`. In the log, the last `PRG start program` says which file loaded; after `Loading sps_new.zip` the
`Program started:` line names the program it replaced, so the `Rename program /lx/prog/<file>` line names the
file that runs. Apply reads it once before `lox config push` and up to four times after it, 3 s apart.
`--no-live` turns it off.

## Pull from Miniserver

**Pull from Miniserver** runs `lox config pull` (the `lox` on `PATH`, or `--lox <path>`), with the same lox
configuration and context as live values. It is offered only when lox would commit to the repository vislox
shows, into the folder of the Miniserver vislox shows (`config_repo` and the serial or address in lox's
configuration). Otherwise the button is greyed out and its tooltip says why. `--no-pull` hides it.

## Roll back

A Roll back (#69) sends the Miniserver nothing Apply does not: the same preflight (serial, FTP, web port), then
`lox config push --patched-from <the Apply's post-push download>` without `--force` (it downloads what the
Miniserver runs and stages the upload; nothing is uploaded), then the same with `--force`, which downloads again
(and refuses unless that is exactly the download the Apply verified), uploads, restarts the program and downloads
again to verify; vislox follows the restart and checks the load as for Apply. Only when that push fails after its
upload line, vislox downloads what the Miniserver runs (`lox config download`, FTP, read-only) and reads `/prog`,
to tell whether anything changed. Working out what it would deploy (`POST
/api/rollback/plan`) reads only the project's `backups/` and git, and runs `lox logic compile` in a scratch
directory: nothing goes to the Miniserver. See [features.md](features.md#apply-sync-and-discard).

Removing managed blocks (#70) adds nothing either: the plan (`POST /api/removals/plan`) and Apply's *Removals
you confirmed* step run `lox logic compile` only, and the push is Apply's (`lox config push --force`, never
`--allow-removals`). Only when that push fails after its upload line, vislox downloads what the Miniserver runs
and reads `/prog`, as for a Roll back.
