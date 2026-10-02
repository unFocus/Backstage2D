# Dev setup

The primary dev host is Bazzite (immutable Fedora) with KDE on Wayland. We
install system libraries with **Homebrew** or a **ujust** recipe, not with
containers or `rpm-ostree` layering.

## Requirements
- Rust stable through rustup. `rust-toolchain.toml` pins the channel and
  components.
- GTK 4 dev files (headers and `.pc`), needed to build `backstage_tools`.
  The system image ships the GTK runtime but not the dev files.
- A Vulkan driver. The system Mesa RADV works; wgpu picks the GPU itself.

## Install
```sh
brew install gtk4 pkgconf
```

The system `/usr/bin/pkg-config` comes first on `PATH` and can't see brew's
`.pc` files, so point the build at brew's pkgconf:

```sh
export PKG_CONFIG=/home/linuxbrew/.linuxbrew/bin/pkgconf
cargo build --workspace
```

If pkgconf complains that `xproto.pc` (or another X11 proto file) is
missing, the xorgproto keg lost its links: `brew unlink xorgproto && brew
link xorgproto`.

## Build time vs run time
- **Build time:** brew's GTK (4.24 when this was written) provides the
  headers and `.pc` files.
- **Run time:** the binary loads the **system** GTK stack (4.22 on
  Bazzite 44). There's no rpath into brew, so brew and system libraries
  are never mixed.
- This works because we compile with gtk4-rs's `v4_14` feature, which
  limits us to APIs that exist in GTK 4.14. Check with:
  ```sh
  ldd target/debug/backstage_tools | grep -E 'gtk|glib|cairo|pango'   # all /usr/lib64
  ```
- If we ever need a GTK API newer than the system's, raise the feature and
  revisit this (add an rpath to brew's lib dir in your own environment, not
  the repo).

## Running
```sh
cargo run -p backstage_tools                          # editor
cargo run -p backstage_player -- samples/bounce.bs2d  # player (no editor)
```
The editor starts `backstage_stage` from the same `target/` directory. The
stage logs the GPU it picked, for example
`backstage_stage: using AMD Radeon RX 6800 (RADV NAVI21) (Vulkan)`.

By default the editor opens the built-in bounce sample. File → Open
(Ctrl+O) opens a project folder; to open one at startup instead:
```sh
BACKSTAGE_PROJECT=samples/bounce.bs2d cargo run -p backstage_tools
```
The editor sends the project to the stage every time the stage (re)starts.
A project that fails to load is shown in a banner, and no stage is started
until one opens.

Runtime files (the socket and frame rings) live in
`$XDG_RUNTIME_DIR/backstage2d/`. Files left by processes that died are
cleaned up the next time the editor starts.

## Editor keys
- **Ctrl+O / Ctrl+S / Ctrl+Shift+S:** open, save, save as (also in the
  header menu). Opening or closing with unsaved changes asks first.
- **Ctrl+Z / Ctrl+Shift+Z:** undo and redo (or the header buttons).
- **Enter:** play or pause the timeline. Dragging on the timeline scrubs.
- **Escape:** clear the selection. **F2** or a double-click renames a layer.
- **Double-click** a composition in the Library, or an instance in Layers,
  to edit that composition; the breadcrumb over the stage goes back.
- **Arrow keys:** a debug edit until on-stage editing arrives (M4). They
  move the root composition's first node (the ground in the sample) by
  10 px, or 1 px with Shift, whichever composition is being edited.

These step aside while a text field or spin button has focus, or a popover
is open. The title shows the project's name, with • while it has unsaved
changes.

## Autosave and crash recovery
Every committed edit is autosaved to
`$XDG_STATE_HOME/backstage2d/recovery/<pid>-<unix time>/` (usually under
`~/.local/state`): `base.bs2d/` is the project as opened, `log.ron` holds
one edit per line, and `meta.ron` records where the project is saved and
how much of the log that covers. The editor prints the directory at
startup and holds a lock on its log while it runs. A clean exit removes the
directory unless there are unsaved changes. At startup the editor offers to
restore the newest directory a crashed editor left with unsaved changes
(Restore, Discard, or Not Now), and deletes leftovers with nothing unsaved.
See [ADR 0004](adr/0004-commands-and-document-authority.md) §5.

## Checks
`scripts/check.sh` runs formatting, clippy, all tests, and the headless
smoke tests for the editor and the player. See [testing.md](testing.md).

## Manual crash-recovery checks
- **Kill Stage** button, or `pkill -9 -x backstage_stage`: the restart
  banner appears, then the stage comes back with a new pid.
- `kill -STOP $(pgrep -x backstage_stage)`: after about 3 s the watchdog
  kills the frozen stage and restarts it.
- Closing the editor: the stage exits and `$XDG_RUNTIME_DIR/backstage2d/` is
  left empty.
- **Editor crash:** make an edit, then `kill -9` the editor's pid (it's in
  the recovery directory's name). The next launch offers to restore it,
  with the undo history and the • in the title.
