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
  compile, Discard, Apply, Sync); it is kept in the browser for the config it was made on, so it survives a reload.
- Fixed: Discard failed when the last commit had no lockfile (the first compile writes one), or one was staged
  but not committed. Discard now reads the last commit, never deletes a lock vislox did not write (one not in git
  goes back to how it was before vislox's first edit), and puts everything back when a step fails.
- Fixed: Discard and the operate buttons ignored mouse clicks.

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
