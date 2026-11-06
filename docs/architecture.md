# How vislox works

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
  also decodes the entries of `calendargetentries` and compares them with a config's (`calendar::drift`), and
  the Miniserver's heating and cooling periods (`Season`, `MM-DD/MM-DD`) into the same yearly range.
- `src/appcalendar.rs` reads the Miniserver's calendar entries (`calendargetentries`) when live values start,
  after Pull, Apply and Sync and before Apply, for the *changed in the app* notice (`GET /api/calendar`).
- `src/actions.rs` runs Apply (`lox config push --force`, `git commit`, `git push`), Sync (`lox config merge`,
  `git commit`, `git push`) and Discard (`git checkout` of the module, lock and out). Apply's steps follow lox's
  progress lines as they come.
- `src/running.rs` reads `/prog` and `def.log` for the config the Miniserver runs (`GET /api/running`), and
  checks after an Apply that the upload loaded.
- `src/edit.rs` is editing from Properties ([editing.md](editing.md)): the edit operations, what each object
  lets Properties edit (`view.objects[…].edit`), and vislox's file of claims in the module
  (`<module>/vislox/edits.loxl`), read and checked with lox's own parser and linter. `actions.rs` writes it and
  runs `lox logic compile` (`POST /api/edit`), and lists what is edited and not deployed (`GET /api/edits`).
- `src/blocks.rs` is adding blocks (#59): the types vislox offers (`GET /api/block-types`, from lox's builtin
  table and its language server's descriptions), and vislox's file of new blocks per page
  (`<module>/vislox/<page>.loxl`). `edit.rs` applies `add_block` and `remove_block` to it.
- `src/restart.rs` follows the program restart during Apply: out of service, back online, new config loaded.
- `src/backups.rs` lists and downloads the Miniserver's own backups for the History and caches their programs
  by blob id; `history.rs` shows each with the Pull of the same bytes, or between the Pulls around it.
- `src/pull.rs` runs `lox config pull` for the Pull button (`POST /api/pull`, which needs an `X-Vislox` header so
  that no other web page can trigger it), then refreshes the history.
- `src/monitor.rs` is the Debug-Monitor: the UDP listener, the address filter, Start/Stop and the Monitor Server
  answer. `src/monframe.rs` parses its frames (checksum, header, parameters, format strings, system stats).
- `src/log.rs` runs `lox log` for the Log panel (`GET /api/log`), splits it into entries, marks the last program
  change and picks out the UUIDs a line names. Its `LoxRead` runs one read-only `lox` command at a time.
- `src/hardware.rs` knows which objects are devices, their kinds, which attributes reach the device and what
  they do there, for the *hardware* tag (`model.rs` marks the changes, `view.hardware` lists the devices), and
  reads the devices on the bus (`lox extensions`, `GET /api/hardware`).
- `src/miniserver.rs` checks the Miniserver for the status chip and the preflight before Pull, Apply and Sync
  (`GET /api/miniserver`), including that the serial at the address is the config's, and when its config last
  changed; `src/configdate.rs` finds the same date for the base, in the download it came from.
- `src/structure.rs` keeps the Miniserver's structure file (`LoxAPP3.json`) for the In the app section
  (`GET /api/app`), caches it and its icons on disk (`GET /api/app/icon`), and reads it again on request.
- `src/route.rs` picks how vislox reaches the Miniserver: its CloudDNS name, lox's host, or Remote Connect
  (`connect.loxonecloud.com/getip`), and explains the Connect Service's codes. `src/tls.rs` builds the HTTP
  and WebSocket clients that check the certificate on a verified route. `loxone-client` cannot do this yet:
  its reqwest has no root certificates (`rustls-tls-manual-roots`), so its `verify_ssl` fails every request,
  and its WebSocket never checks. vislox swaps the HTTP client inside lox's `LoxClient` (which keeps
  `verify_ssl`, so a client lox might build from it fails rather than skip the check) and opens the WebSocket
  itself. lox's login fetches the RSA key with a client of its own that sends the user and password: vislox
  fetches the key itself, checked and without credentials, and gives lox's login a one-off loopback address
  that serves it, so that request never leaves this host.
- `src/discover.rs` is Loxone's UDP discovery (`vislox --discover`, and finding a Miniserver whose address
  changed).
- `src/operate.rs` sends the commands of `--operate` to virtual inputs (`POST /api/operate`, with the admin
  token) and keeps the command log.
- `src/live.rs` streams the Miniserver's states with lox's `loxone-client` crate. A wire's source
  connector has the UUID of the state it carries, so the browser matches values to wires by UUID. Values
  reach the browser on `/api/live` (server-sent events). `src/query.rs` maps a queried block's answer to its
  connectors, and `model::query_plan` picks the blocks a page needs.
- `src/session.rs` is one WebSocket session (connect, log in, subscribe, stream, the opt-in block queries and the
  heating and cooling periods),
  built from `loxone-client`'s parts so that it keeps the Miniserver's close code. `src/reason.rs` turns close and response codes into a
  reason, a plain sentence and a backoff.
- File changes are noticed with `notify`. Git is polled every 3 s. Browsers get a server-sent event and
  reload.


## Other notes

- [program-memory.md](program-memory.md) records why vislox does not estimate a change's program memory from
  `/sys/Sizes.bin` (on the Gen 2 Miniserver checked, firmware 17.2.10.21, the file holds a constant, not sizes).
- The look is based on Loxone Config screenshots, which are copyright Loxone and are not part of the repository.
  `.gitignore` excludes `docs/reference/`, so a local copy for comparison is never committed.
- `VISLOX_FTP_PORT` overrides the FTP port the status check connects to (default 21). It exists for tests;
  lox always uses 21.
