<img src="assets/icon.svg" width="64" height="64" alt="">

# vislox

vislox shows a Loxone Miniserver config in the browser the way Loxone Config draws it, and marks every change
so you can review it before you deploy. It works with the configs that [lox](https://github.com/discostu105/lox)
manages, reloads when the file changes, and is read-only unless you turn on Apply or `--operate`.

## Quick start

With Docker (the image bundles git and lox; see [docker/README.md](docker/README.md) for all settings):

```bash
docker run -d -p 7878:7878 -v ~/loxone-config:/data/repo ghcr.io/hakubo/vislox:main
```

From source (Rust 1.91 or later; `lox` on `PATH` for live values, Pull, Apply and Sync):

```bash
cargo install --path .
vislox ~/loxone-config          # a `lox config init` repository (<serial>/config.Loxone)
vislox ./my-project             # a lox.toml project: deployed base vs compiled out.Loxone
vislox config.Loxone            # one .Loxone file, or a backup .zip
vislox --discover               # list the Miniservers on the local network
```

Then open <http://127.0.0.1:7878>. Started without a path or `--ctx` in a directory with no config (and without
`--no-live`), vislox lists the Miniservers on the network and how to set one up with lox.

## Features

- Logic pages as tabs, with blocks, references and wires in the Loxone Config 16 look, using lox's own layout.
- Change review: added, changed, removed and moved blocks, wires and parameters, against any earlier revision.
- History: the pending compile, snapshots taken while vislox runs, git commits, Pull from Miniserver, and the
  Miniserver's own backups of each save from Loxone Config.
- Which config the Miniserver runs (from `def.log` and `/prog`), with a warning when an upload is still waiting,
  it runs its emergency config, or the newest file was skipped; Apply checks that its upload loaded.
- Live values on the wires, as in Loxone Config's Liveview, with opt-in queries for blocks the app does not see.
- Properties for the selected block: connections, parameters, Daytimer schedules, calendar entries, what the
  Loxone app shows, and the hardware a change reconfigures.
- Operating modes as a year calendar, with the heating and cooling periods.
- A Miniserver status chip and a preflight before Pull, Apply and Sync (right serial, FTP and web port reachable,
  config changed since the base, calendar entries changed in the app).
- The Miniserver log and, opt-in, its Debug-Monitor.
- For a `lox.toml` project: edit values in Properties, add blocks to a page, move the blocks the module manages and
  draw wires, then Apply, Sync and Discard from the browser.
- For a `lox.toml` project: the blocks its Lox Logic module manages marked, each with the `.loxl` file and line that
  makes it.
- For a `lox.toml` project: `lox logic check` errors listed by file, line and block when the module doesn't compile
  (Apply is off until it does), and *Compile* when the pending config is from before the module's latest change.
- For a `lox.toml` project: `lox logic lint` findings (unused names, blocks whose outputs feed nothing, users whose
  tag opens no Code Touch, …) as a warning badge on each block and a *Lint* list in the Deploy pane. Hints only.
- For a `lox.toml` project: *Roll back* an Apply from the History, onto the download `lox config push` kept from
  before it, through Apply's own steps and checks.
- For a `lox.toml` project: when the module no longer has a block it manages, the removal is reviewed before Apply
  (what it deletes, where, what is wired to it, who loses access) and deployed only with consent to exactly that list.
- For a `lox.toml` project: the module's `lox logic test` scenarios run in lox's simulator, each with pass, fail or
  couldn't run, the failing `expect` with expected and actual values, and the blocks it checks. Apply stops while
  a test fails.
- Ctrl+K to find any page or block (by name, or by its Lox Logic slug), links that reopen a page, block and revision, and full keyboard use (`?`).

Details: [docs/features.md](docs/features.md).

## Configuration

vislox uses lox's Miniserver connection (`lox setup set …`), found the way lox finds it: `LOX_CONFIG`, a
project's `.lox/config.yaml`, then `~/.lox/config.yaml`. With several contexts it picks the one for the
Miniserver it shows (the repository's `<serial>` or `192_168_1_77` folder), else the active one.

| Flag | Environment | |
|---|---|---|
| `[PATH]` | | A lox.toml project (or its directory), a `.Loxone` file, a backup `.zip`, or a `lox config init` repository. Default: the current directory. |
| `--serial <SERIAL>` | | In a repository with several Miniservers: which one. |
| `--listen <ADDR>` | | Address to serve the UI on. Default `127.0.0.1:7878`. |
| `--open` | | Open the UI in the default browser. |
| `--discover` | | List the Miniservers on the local network (UDP discovery), then exit. |
| `--ctx <NAME>` | | The lox context for live values. |
| `--route <auto\|local\|remote>` | `VISLOX_ROUTE` | How vislox reaches the Miniserver. `auto` (default): its CloudDNS name with the certificate checked, else lox's host, else Remote Connect. `local` never uses Remote Connect; `remote` always does. |
| `--no-live` | | Never connect to the Miniserver: no live values, status checks, log, structure file or backups reads. |
| `--live-from <FILE>` | | Live values from a file (or `-` for stdin) of `lox stream -o json --initial` lines, e.g. a recording. |
| `--no-pull` | | Hide *Pull from Miniserver*. |
| `--lox <PATH>` | | The lox binary. Default `lox`. |
| `--admin-token-file <FILE>` | | A file holding the passphrase that Apply, Sync and Discard ask for. Without it they are off. |
| `--operate` | | Let the browser send commands to virtual inputs. Needs `--admin-token-file` and live values from the Miniserver. |
| `--monitor` | `VISLOX_MONITOR=1` | Receive the Miniserver's Debug-Monitor (UDP). |
| `--monitor-port <PORT>` | `VISLOX_MONITOR_PORT` | Its UDP port. Default `7777`. |
| `--monitor-ip <IPV4>` | `VISLOX_MONITOR_IP` | The address the Miniserver sends it to. Default: this host's address on the route to the Miniserver. |
| `--monitor-minutes <N>` | `VISLOX_MONITOR_MINUTES` | How long a Start lasts. Default `60`. |
| `--monitor-strings <FILE>` | `VISLOX_MONITOR_STRINGS` | The Miniserver's string table (`ENG.xml`), for lines sent as a string ID. |
| | `LOX_CONFIG` | lox's config file, as lox reads it. |
| | `VISLOX_CACHE` | Cache for the structure file, icons and backups. Default `$XDG_CACHE_HOME/vislox`, else `~/.cache/vislox`. |

```bash
vislox ~/loxone-config --serial 504F94AABBCC --ctx home    # one Miniserver of several, with a named lox context
lox stream -o json --initial --reconnect | vislox ~/loxone-config --live-from -   # live values from lox
vislox config.Loxone --live-from recorded.ndjson           # replay a recording (followed as it grows)
```

The Docker image takes its own variables (git URL, lox credentials, admin token, …): see
[docker/README.md](docker/README.md).

### TLS and Remote Connect

With the Miniserver's serial known, vislox connects to a local Gen 2 Miniserver under its CloudDNS name and checks
its certificate. Away from home it goes through Loxone's Remote Connect (firmware 17.0 and later), where live
values and the status chip work but Pull, Apply and Sync are refused (FTP does not pass the relay). lox's own
commands always use lox's host. Details: [docs/miniserver.md](docs/miniserver.md#route-tls-and-remote-connect).

## Safety

By default vislox only reads. It never writes to a config file itself: lox and git do, on Pull, Apply, Sync and
Discard. Editing values and adding blocks write files of vislox's own into a project's module (`<module>/vislox/`),
and moving a block writes its position into the module's lockfile; `lox logic compile` turns them into the pending
config ([docs/editing.md](docs/editing.md)).

Without any opt-in, vislox sends the Miniserver read-only requests: the live value stream, the structure file and
its icons, the status check, the heating and cooling periods, the calendar entries, the system log (`lox log`),
the devices on the bus (`lox extensions`), which program file it runs (the `/prog` listing and `def.log`), and
the backup list and downloads (FTP) when you click *Read*. Pull
runs `lox config pull`, which downloads the config. When lox's address does not answer (or with
`--route remote`), vislox asks Loxone's `connect.loxonecloud.com` for the Miniserver's Remote Connect relay. `--no-live` stops all of these except Pull.

Opt-ins:

| What | Turned on by | Sends to the Miniserver |
|---|---|---|
| Apply | `--admin-token-file`, then the token in the browser | `lox config push --force`: upload and program restart |
| Roll back | `--admin-token-file`, then the token | `lox config push --force --patched-from …`: upload and program restart, as Apply |
| Sync, Discard | `--admin-token-file`, then the token | nothing; they change the project and its git repository |
| Compile (a module changed outside vislox) | `--admin-token-file`, then the token | nothing; it runs `lox logic compile` |
| Edit values, add and move blocks, draw wires | `--admin-token-file`, then the token | nothing; vislox writes its files in the module (and positions in the lockfile) and runs `lox logic compile` |
| Operate virtual inputs | `--operate` with `--admin-token-file` | `jdev/sps/io/<uuid>/<cmd>`, only to `VirtualIn` objects, each confirmed |
| Query blocks on this page | a toggle in the legend, off by default | read-only `jdev/sps/io/<uuid>/all`, at most 20 a second |
| Debug-Monitor | `--monitor`, then Start | `dev/sps/log/<address>`: the Miniserver sends its debug output to vislox until Stop or the time limit |

Only Apply (and Roll back, which is an Apply) and operate commands change the Miniserver's program or its state.

Before Pull, Apply, Roll back, Sync and each operate command, vislox checks that the Miniserver at lox's address has the
config's serial, and refuses otherwise. Every request is described in [docs/miniserver.md](docs/miniserver.md).

## Documentation

- [docs/features.md](docs/features.md): every feature in detail, and the keyboard shortcuts.
- [docs/miniserver.md](docs/miniserver.md): every request to the Miniserver, caching, error codes and retries.
- [docs/architecture.md](docs/architecture.md): how the code is organised.
- [docs/debug-monitor.md](docs/debug-monitor.md): the Debug-Monitor, and what was found out about its protocol.
- [docs/editing.md](docs/editing.md): editing in vislox, how edits reach the module, and what comes next.
- [docs/agent-workflow.md](docs/agent-workflow.md): changing the config with your own agent (proposal).
- [docs/program-memory.md](docs/program-memory.md): why vislox does not estimate program memory.
- [docker/README.md](docker/README.md): the Docker image, Compose and Portainer.

## Development

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
cd tests/ui && npm ci && npx playwright install chromium && cd ../..
VISLOX_BIN=target/release/vislox node tests/ui/smoke.mjs   # drives the UI in headless Chromium
VISLOX_BIN=target/release/vislox node tests/ui/apply.mjs   # Apply's steps, with a stand-in lox
VISLOX_BIN=target/release/vislox node tests/ui/edit.mjs    # editing values, adding and moving blocks, with a stand-in lox
VISLOX_BIN=target/release/vislox node tests/ui/rollback.mjs  # rolling back an Apply, with a stand-in lox
```

CI (`.github/workflows/ci.yml`) runs all of these on every push to `main` and every pull request.

## Versions

The version is in `Cargo.toml`; changes are listed in [CHANGELOG.md](CHANGELOG.md). `vislox --version` and the
UI (next to the logo) show it with the commit it was built from. To release, bump the version in
`Cargo.toml`, move the changelog's *Unreleased* entries under it, merge, then tag the merge commit `vX.Y.Z`
and push the tag. The Docker workflow publishes the image as `X.Y.Z`, `X.Y` and `latest`, and refuses a tag
that does not match `Cargo.toml`.

## Licence

[MIT](LICENSE)
