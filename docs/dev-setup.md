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

By default the editor opens the built-in bounce sample. To open a
different project directory:
```sh
BACKSTAGE_PROJECT=samples/bounce.bs2d cargo run -p backstage_tools
```
This is a stopgap until File → Open (M3). The editor loads the project and
sends it to the stage every time the stage (re)starts. A project that fails
to load is shown in a banner, and no stage is started.

Runtime files (the socket and frame rings) live in
`$XDG_RUNTIME_DIR/backstage2d/`. Files left by processes that died are
cleaned up the next time the editor starts.

Every committed edit is autosaved to
`$XDG_STATE_HOME/backstage2d/recovery/<pid>-<unix time>/` (usually under
`~/.local/state`): `base.bs2d/` is the project as opened and `log.ron` holds
one edit per line. The editor prints the directory at startup. A clean exit
removes it only if it holds no edits; until Save exists (M3), a directory
with edits is the only copy of that work. Restoring it from the editor also
comes with M3.

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
