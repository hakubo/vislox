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
- `src/rights.rs` reads who may open each NFC Code Touch and who gets each push notification from the users
  and groups of a `.Loxone` file (lox's `AccessModel`), directly and in effect (through groups), and compares two
  configs block by block for the review's *Rights and schedules* and Apply's confirmation (#68). Only titles leave
  it. `managed.rs` finds the module's `access`, `recipients` and `schedule` claims and their lines
  (`set_claims` of `GET /api/edits`).
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
- `src/rollback.rs` finds the downloads `lox config push` leaves in a project's `backups/` (`<stamp>-pre-push` and
  `-post-push`) for Roll back (#69): a commit whose base is a post-push program is a push, and its pre-push
  program is what ran before. It reads and checks them; `actions/rollback.rs` plans the Roll back (`POST
  /api/rollback/plan`, a scratch compile of the module from before, under the compile lock) and runs it through
  Apply's steps (`POST /api/rollback`), checking that the in-place compile, lox's staged upload and its verified
  download are the reviewed program, and working out what runs when a push fails after its upload. `history.rs`
  keeps the plan as the `rollback:<commit>` revision, found by id only (the asking tab shows it).
- `actions/removals.rs` handles blocks a push would delete (#70): lox's refusal for a block the module lost, and
  lockfile tombstones for blocks the base has. It plans a compile with `--allow-removals` in a scratch directory
  (`POST /api/removals/plan`, `POST /api/removals/cancel`; `model::removed_blocks` says where each block is, what
  is wired to it and whose rights go with it), and is Apply's *Removals you confirmed* step: without consent it
  refuses (from the check when it is current), with it it compiles the plan into the project with the plan's
  `--time` and returns a `PutBack` that restores the lockfile and `out` unless the push goes through. A push
  that fails after its upload line is judged by `Actions::what_runs` (in `actions.rs`, shared with Roll back):
  the pushed program is kept as deployed, the previous one with nothing waiting is put back, anything else is
  reported as unknown.
  `history.rs` keeps the newest plan as the `removals:<blob>` revision, compared with the deployed revision,
  found by id only, and drops it with its bytes when it is cancelled, replaced, too old or another revision runs.
- `actions/scan.rs` checks what a git push would send for personal data (#71): the branch's upstream
  (`target`), the objects a push of `HEAD` sends there (`git rev-list --objects <sha> --not` the remote's tracking
  branches), and of those each config file version (`.Loxone`, `.LoxCC`, a backup `.zip`'s program), copied into
  a private scratch directory and read by `lox logic scan --json`, each finding given the line of the element lox
  names. `Actions::push_git` (in `actions.rs`) is the only place vislox runs `git push`, and pushes exactly the
  scanned commit (`git push --no-follow-tags <remote> <sha>:<branch>`): Apply, Roll back and Sync go through it,
  as Apply's *No personal data in git* step, and *Push to git* (`POST /api/push`) retries it. A finding, a file lox
  cannot read or no answer blocks the push and is kept as `actions.push` (at most 50 findings) for the Deploy pane;
  the git poll forgets it once the upstream has every commit (`Actions::recheck_push`).
- `src/running.rs` reads `/prog` and `def.log` for the config the Miniserver runs (`GET /api/running`), and
  checks after an Apply that the upload loaded.
- `src/edit.rs` is editing from Properties ([editing.md](editing.md)): the edit operations, what each object
  lets Properties edit (`view.objects[…].edit`), and vislox's file of claims in the module
  (`<module>/vislox/edits.loxl`), read and checked with lox's own parser and linter. `actions.rs` writes it and
  runs `lox logic compile` (`POST /api/edit`), and lists what is edited and not deployed (`GET /api/edits`).
- `src/sched_edit.rs` is editing a Daytimer's switching times (#73): the `schedule` op's value (columns by
  operating-mode UUID), checked as lox would (minutes, `24:00`, overlaps, an analog timer's values), written as
  `schedule vx_… end` sets and `.schedule` claims in `edits.loxl` (the columns spelled as lox takes them: day
  word, the module's slug of a mode, else its title; alike columns on one line, `daily`/`weekdays`/`weekend`), read
  back into columns (an op's `previous`), and checked after the compile (`missed`). `edit.rs` keeps them in
  `Claims` and dispatches the op; `GET /api/edits` sends `schedules` and `schedule_sets`.
- `src/set_claim.rs` is what every shared-set claim (switching times, moods, access, recipients and tiles) has in
  common: the op's resolve, choose-and-check, claim, prune and whole-inverse undo (`apply`), and
  `GET /api/edits`' section (`edits_section`), parameterised by a `SetKind`. `sched_edit.rs` and `mood_edit.rs`
  implement it.
- `src/mood_edit.rs` is editing a lighting controller's moods (#74): the `moods` op's value (each mood's name and
  its values in Lox Logic's terms, read with lox's parser), checked against every controller on the set as lox
  would (the off mood, the built-in and motion moods, the values each circuit's kind takes, with lox's `moods`
  codec), written as `moods vx_… end` sets and `.moods` claims in `edits.loxl`, and checked after the compile
  (`missed`); also the `Circuit<n>:` setting of a controller added or adopted here. `model.rs` reads a controller's
  circuits and moods for Properties (`moods`, `moods_old`) and the Changes list; `GET /api/edits` sends `moods` and
  `mood_sets`.
- `src/member_edit.rs` is editing a central block's members (#75): the `members` op (the member UUIDs in order),
  checked as lox would (the type a CentralLight or CentralShade takes, each once, never itself; an alarm's
  `Objects=` read-only), written as `.members` claims in `edits.loxl` (externs for Loxone Config's blocks, the slugs
  of vislox's own), checked after the compile (`missed`), and `GET /api/edits`' `members`. A member list is no
  shared set, so it is not a `SetKind`; it takes `set_claim.rs`'s naming of vislox's blocks. `model.rs` reads the
  members for Properties (`members`, `members_old`) and the Changes list; `managed.rs` finds the module's own
  `.members` claims (`members_claims`).
- `src/access_edit.rs` is editing who may open an NFC Code Touch (#76): the `access` op (users and groups by UUID,
  each with its access schedule's UUID), a `SetKind` of `set_claim.rs` written as `access vx_… end` sets and
  `.access` claims in `edits.loxl`, each title checked to name exactly one user, group or schedule (lox's
  `AccessModel`, no credential read), checked after the compile against `rights.rs`' reading of the compiled config
  (`missed`), and `GET /api/edits`' `access` and `access_sets`. `Directory::choices` is the pickers' list, served
  only to a request with the admin token (`POST /api/access/choices`, from the revision whose blob the draft is
  made on). The review and Apply's confirmation are #68's `rights.rs`, which also says who keeps a door under
  fewer access schedules (`narrowed`, a loss).
- `src/recipients_edit.rs` is editing who gets a push notification (#77): the `recipients` op (users and groups by
  UUID, no schedule), a `SetKind` of `set_claim.rs` written as `recipients vx_… end` sets and `.recipients` claims,
  read with `access_edit.rs`' `Directory` (the same pickers' list, which also names the Full access groups), checked
  after the compile against `rights.rs` (`missed`), and `GET /api/edits`' `recipients` and `recipient_sets`.
  `rights.rs` names the users who get a notification only through Full access (`inherited`). A notification's block
  on a page (`OutputRefLM`) is `model.rs`' `message`/`message_old` (its `<LoggerMailer>`), and is added with
  `add_block` (`mirrors:` an extern of the `Notification`, `blocks::MESSAGE_REF`) and edited as a block of
  vislox's files (`On`, `Off`, `MinimumTime` as settings).
- `src/tiles.rs` reads a Touch Pure Flex's tiles (`<btns>`: each tile's click, up and down, with command, status
  text and `ctrls=`) and the blocks API-linked to each device (`Links`, lox's own reading: `OutputAPI` wired to the
  device's `ApiActor.I`, directly or through an `OutputRef` of it, either direction), for `model.rs` (`tiles`,
  `tiles_old`, the Changes list) and the editor. `src/tiles_edit.rs` is editing them (#78): the `tiles` op, a
  `SetKind` of `set_claim.rs` written as `tiles vx_… end` sets (in lox's own format, `Module::to_text`) and `.tiles`
  claims, its targets externs of vislox's file (`SetKind::lines_with`, `SetKind::named`), checked against the
  device's layout, its API links and the types lox knows (`As`, `Lico`), and after the compile (`missed`).
  `check::pin_tile_findings` pins lint's tile findings to a device named by its title.
- Block settings (#79) are values of a block of vislox's files (`edit.rs`' `edit_new_block`, `blocks::settings`
  and `Kind`): a program is `Kind::Program`, a `Value::File` whose text `Files` reads from and writes to
  `<module>/vislox/<slug>.c` (`edit::ours` says which files of vislox's directory are its). `src/units.rs` shows a
  constant with the unit Lox Logic writes it with and checks a typed unit against the port (`units::shown`,
  `units::port_value`). `model.rs`' `settings` (`SettingInfo`, `setting_text`) and `attr_change_text` read a
  block's settings by their Lox Logic names, for Properties and the Changes list.
- Deleting and renaming a block of vislox's files (#80) are `edit.rs`' `delete_block` (a deployed or adopted
  block: `removed`, and a `restore_block` undo carrying its lockfile entry from `Lock::entries`), `restore_block`
  (the entry back and the tombstone out, through `adopt::LockEdits`' `add`, `untomb` and `wires`) and
  `rename_block` (lox's `rename_slug` over vislox's files, read back with `Files::from_texts`, and `moved`).
  `blocks::PageFile` keeps the `removed` and `moved` lines; `Files::tidy` drops those lox needs no more, and
  `Files::stale_lines` lists them. `actions.rs` checks the compiled config: a deleted block gone, a restored one
  back under its UUID, a rename changing nothing (`loxone_config::diff`).
- `src/constants.rs` reads the module's `let` constants and `defaults` statements and the ports that take a value
  from one (#81), with lox's parser and `defaults_taken`, for `GET /api/edits`' `constants`. vislox's own constants
  are `Claims`' ninth field in `edit.rs` (`let vx_… = …` in `edits.loxl`), its defaults the eleventh (`defaults
  T(P: v)`); the `let` op is `set_constant`, and a `param` op whose value names a constant (`Ctx::lets`, the
  module's) binds the port to it (`constant_ref`). `constants::reach` is the blocks a default reaches (the module
  expanded without the type's statement), and `constants::pending` the values that differ from the deployed
  revision's files. `src/module_edit.rs` rewrites one value of the module's own `let` or `defaults` line
  (`set_let`, `set_defaults`; `Ctx::module` holds the module's files, `Files::module` the rewrites, which
  `actions.rs` writes with vislox's files and puts back on failure) and makes vislox's `defaults`, checking the
  op's `source` and `blocks` first.
- `src/expressions.rs` reads the module's expressions and named signals (#82) with lox's parser: what each feeds,
  its text as lox prints it, its file and line, and its blocks (the lockfile's expression-owned entries under its
  prefix, `<sink>_<port key>__` or `<signal>__`, its top block named by lox's post-order count), and what the
  deployed config had (the block feeding the sink, or the signal's by UUID, and its label), for `GET /api/edits`'
  `expressions`. `src/expr_edit.rs` is editing them in vislox's files: the `expr` op (an expression or a port of
  vislox's into one input: `Claim::exprs` in `edits.loxl`, `NewBlock::exprs` in a declaration, or a wire) and the
  `signal` op (`PageFile::signals`, with `.title`), operands checked against vislox's names.
- `src/templates.rs` reads the module's templates and their instances (#83) with lox's parser: each template's
  parameters and outputs, each instance (nested ones by their path, `sued.hitze`) with its arguments as written, the
  lockfile entries it made (`<prefix>_<body name>`, and lox's expression blocks of the body), its outputs' blocks,
  and against the module's files at the deployed revision's commit, whether a template's body or an instance's call
  changed; and lox's template library (`loxone_config::library`), for `GET /api/edits`' `templates`.
  `src/template_edit.rs` is the `instance` op: a call in vislox's page file (`blocks::InstanceDef`, under the
  file's *Template instances* comment), checked against the template's parameters, its library template copied into
  `<module>/vislox/templates.loxl` (`Files::templates`) while it is called, `removed` lines for a dropped compiled
  one. The module's own templates reach the op as `Ctx::templates` (`edit::module_decls`).
- `src/periphery.rs` is the objects of the object tree vislox adds (#84): its table of lox's periphery types (where
  each lives, the settings a new one needs, which take text; a test holds it to lox's public tables), where a new
  one goes (`parent`: its folder, or a device of vislox's or Loxone Config's), and the palette's `periphery`. They are
  declared in `<module>/vislox/_periphery.loxl`, a `PageFile` with `periphery` set and no page line; `edit.rs`'
  `add_block` hands them to `add_object`, and `remove_block`, `restore_block` and `edit_new_block` take them as blocks.
- `src/hw_edit.rs` is Tree and Air hardware (#85): `<module>/vislox/_hardware.loxl` (`Files::hardware`, an
  `HwFile`: the Tree extensions it names, devices by `uuid:` or created with `kind:`, I/O by `uuid:` or `iname:`
  with `Statistics:`, `removed` lines), the `statistics` op, `add_block` of a `TreeDevice` (`add_device`), and
  `remove_block`/`restore_block` of a created device, whose lockfile entries (`externals`, `hardware`) `adopt.rs`'
  `LockEdits::created` writes back. `edit::check_module` validates the expanded module, where a declaration is an extern.
- `src/calendar_edit.rs` is operating modes and calendar entries (#86): a `Mode` and a `CalendarEntry` are
  `periphery.rs` types in `_periphery.loxl`; an entry's mode (`switches:`, spelled by `sched_edit::Modes::spell`) and
  date (`every:`, `easter:`, `from:`/`to:`, `DateArgs`) are the `entry_mode` and `entry_date` value ops, read back from
  the config by `edit::current` (`mode_of`, `date_of`), and `GET /api/edits`' `entries` says which entries are edited
  here. `web/modes.js` has the form and the picking on the year.
- `src/page_edit.rs` is new logic pages (#88): `new_page` (a file of vislox's headed `page "…" (room: …, category:
  …)`, the room and category checked against the base as lox resolves them), `remove_page` (while not deployed and
  empty), `target` (the page a block, signal or instance of vislox's goes on: a page of the base, or one created here,
  found by the UUID lox keeps in the lockfile's `pages`, `edit::Lock::pages`) and `GET /api/edits`' `pages`.
  `hand_off` drops `room:` once the base or the deployed revision has the page; `actions.rs`' `hand_off_pages` runs
  it (and `follow_renames`) on disk before every compile vislox starts and before Apply's and Sync's commits.
- `src/rules.rs` reads what an Automatic Designer rule does (#87) with lox's decompile (its own reading of the `APC=`
  payload and the app's capability table are private to lox): each rule Lox Logic writes as lines (`Line`: conditions,
  `or`, actions, blocks and modes by UUID), why not for the others; the words and the one-sentence summary Properties
  shows, and a changed rule's lines for the Changes list (`model.rs`' `rule`, `rule_old`). `src/rule_edit.rs` is
  editing them: the `rule` op (a `rule vx_<uuid> … end` declaration on an extern of `edits.loxl`, its lines turned
  into Lox Logic and read back with lox's parser), `new_rule` (`rule vx_rule_<n> "Name"`, which lox creates and pins
  in the lockfile's `rules`), `remove_block` and `restore_rule` of a rule created here (`removed`, its tombstone back
  through `adopt::LockEdits::rules`), a `ClaimKind` of `set_claim::KINDS` (`Claims`' tenth field), and
  `GET /api/edits`' `rules` and `rule_edit` (which rules vislox may state, whether one can be created, the states and
  commands the config's rules use per block type). `managed::rule_claims` finds the module's own declarations.
- `src/set_claim.rs`' `KINDS` lists every kind of claim on a block's whole content (switching times, moods,
  access, recipients, tiles, members) as a `ClaimKind`; `edit.rs` and `actions.rs` loop over it instead of naming each kind.
- `src/blocks.rs` is adding blocks (#59): the types vislox offers (`GET /api/block-types`, from lox's builtin
  table and its language server's descriptions), and vislox's file of new blocks per page
  (`<module>/vislox/<page>.loxl`). `edit.rs` applies `add_block` and `remove_block` to it.
- `src/adopt.rs` is adopting a block Loxone Config drew (#72): it turns what `lox logic adopt` wrote on a scratch
  copy of the module and lock into vislox's files (`extract`: the declaration and its wires for the page file,
  lox's externs renamed to vislox's `vx_<uuid>` ones, the lockfile entry and tracked wires), edits the lockfile
  (`write_lock`, also for the undo: an entry dropped with every `extern_wires` entry of its ports), picks and
  checks slugs, and checks the compile (`verify`: the pending config unchanged, each block exactly the base's).
  `actions/adopt.rs` runs lox on the scratch copy for the `adopt` and `adopt_page` ops, sources before the blocks
  they feed, and says why a block is not offered (`adopt` in `GET /api/edits`); `edit.rs` applies the ops.
- `src/managed.rs` finds where the module's `.loxl` files make each block the lockfile manages (#64): its
  declaration, the expression lox made it for, or the template instance, with lox's own declaration scan.
  `GET /api/edits` sends it as `sources`.
- `src/check.rs` reads `lox logic check --json` and `lox logic fmt --check` (#65) and points each error at the
  blocks its line names; it reads `lox logic lint`'s findings (#66) too and points each at its declaration and
  the block it names, and `lox logic test`'s results (#67), each test at its line and the blocks its `expect`s
  read. `actions.rs` runs them in a check worker when the module, lock, base or out changes
  (`GET /api/check` reads the result), makes *Module compiles* and *Tests pass* Apply's first steps, and compiles on request
  (`POST /api/compile`). Every compile and write of the project holds one compile lock. The module's directory is
  watched recursively, so a change there is checked at once.
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

Object-tree adoption uses the same scratch-module path as page blocks. `adopt::extract` maps `device:` and `switches:` as values, preserving their references, and files declarations without pages in `vislox/_periphery.loxl`. Device descendants are validated against the base before joining the adoption and its inverse. The server supplies adoption refusals to the UI.
