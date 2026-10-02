# Backstage2D — working notes for Claude

A Flash Pro–inspired 2D animation engine and editor in Rust. Read
`docs/roadmap.md` for where things stand, and the ADRs in `docs/adr/` before
changing anything architectural.
- ADR 0001: GTK 4 + Relm4.
- ADR 0002: the stage and tools process split.
- ADR 0003: the document model, the evaluated scene, and time.
- ADR 0004: edit commands, the document log, and document authority.

## Environment
- **Dev host:** Bazzite (immutable Fedora 44), KDE on Wayland, AMD RX 6800.
- **Installing system libraries:** use **Homebrew or ujust**, never
  distrobox, toolbox, or `rpm-ostree`. Check what's available with
  `brew info` / `ujust --list` rather than assuming.
- **Building the GTK crate:** export
  `PKG_CONFIG=/home/linuxbrew/.linuxbrew/bin/pkgconf` first (the system
  pkg-config can't see brew's GTK). `scripts/check.sh` does this for you.
- GTK is built against brew's 4.24 headers but runs on the system's 4.22.
  Keep gtk4-rs's `v4_14` feature; don't raise it without revisiting
  `docs/dev-setup.md`.

## Commands
```sh
scripts/check.sh                                   # everything CI runs; must pass before committing
cargo run -p backstage_tools                       # editor (starts backstage_stage itself)
cargo run -p backstage_player -- samples/bounce.bs2d
cargo run -p backstage_core --example scene_dump -- samples/bounce.bs2d 0.5s [--crossfade squash@1s/0.2s]
BACKSTAGE_BLESS=1 cargo test -p backstage_render --test golden        # re-bless golden images
BACKSTAGE_BLESS=1 cargo test -p backstage_core sample_matches        # re-bless the sample project files
cargo test -p backstage_stage --test stage_protocol -- --ignored --nocapture latency
```
- `BACKSTAGE_WGPU_FALLBACK=1` forces the software renderer (lavapipe). Tests
  set it.
- `BACKSTAGE_PROJECT=<dir>` makes the editor open that project at startup
  instead of the built-in sample. File → Open (Ctrl+O) sends another one to
  the running stage with `Load`; the stage takes no project argument.
  Save is Ctrl+S, and Save As is Ctrl+Shift+S. Enter plays or pauses the
  timeline, Escape clears the selection, and F2 (or a double-click) renames
  the selected layer. Window shortcuts step aside
  for focused text fields, spin buttons, and popovers
  (`keys_belong_to_focus` in `app.rs`).
- The editor owns the playhead and sends it with `ToStage::Transport`
  (protocol v3). The editor's stage starts paused at 0.

## Architecture rules (enforced by `dependency_boundaries.rs`)
- `backstage_core` is pure data and logic. No GPU, GUI, windowing, or lyon.
- The tools crate (GTK) never links wgpu, lyon, the renderer, or the script
  crate. It talks to the stage only through `backstage_protocol`.
- Runtime crates (core, protocol, render, script, stage, player) never
  depend on the tools crate, GTK, or Relm4.
- The renderer draws `evaluate`'s `Scene`. It never samples animations
  itself.
- Platforms are Wayland, macOS, and Windows 11. No X11 (winit is built
  Wayland-only). No GPL dependencies.
- **Licensing is undecided:** `LICENSE` says all rights reserved, and every
  crate has `publish = false`. Don't add license headers or change that
  without the user deciding.

## Model conventions (ADR 0003)
- **Time is `Time` in flicks, never frames.** A 2 s tween takes 2 s at any
  frame rate. Snapping grids are editing aids only.
- **Files are canonical RON:**
  - time strings like `"0.5s"` or `"1/60s"`
  - colors like `"#rrggbb"`
  - angles in degrees; y is down
  - IDs like `node_0000002x12401`

  A format change shows up as a diff of `samples/bounce.bs2d/`. Bump
  `FORMAT_VERSION` if old files would break.
- Protocol changes show up in the insta wire snapshot. Bump
  `PROTOCOL_VERSION` with them.
- Core types never go through postcard directly: they skip default fields,
  which postcard can't read back. In protocol messages, document data uses
  `#[serde(with = "ron_text")]` or a `Snapshot` (the project's files).
- `evaluate` must stay pure and deterministic. An empty `RuntimeState` is
  valid.

## Editing conventions (ADR 0004)
- **Every edit is an `Entry`** (`Do(Command)`, `Undo`, `Redo`) that the
  stage commits. The editor never changes its `WorkingCopy` directly; it
  submits and applies only `Committed` entries.
- **Commands never generate IDs.** The caller picks them, so replay is
  deterministic.
- **A new command needs:** an exact inverse (the round-trip tests compare
  whole projects), lookups before any change, and a case in the
  random-edit proptest picker (`backstage_core/src/test_util.rs`).

## How we work
- **Plan each step first** (plan mode), get approval, then build. Keep steps
  small: roughly one roadmap bullet each.
- **Every change comes with tests in the right tier** (see
  `docs/testing.md`). Temporarily break the code once to prove a new
  regression test fails.
- **Look at rendered output before blessing goldens.** Visual review has
  caught a real bug that the passing tests would otherwise have locked in.
- **Keep docs current in the same commit:**
  - Check off roadmap items.
  - Record decisions in an ADR.
  - Update `docs/testing.md` and `docs/dev-setup.md` when tests or setup
    change.
- **Commits go to `main`** (solo project) with a descriptive message, then
  get pushed. CI (`.github/workflows/ci.yml`, Fedora 44 container) runs
  `scripts/check.sh`. Watch it with `gh run watch`.
- **When checking the GUI on the desktop,** run it with `setsid`,
  screenshot with `spectacle -b -n -a -o <file>`, and clean up afterwards.
- **Headless UI runs** use `cage` with `WLR_BACKENDS=headless`.

## Gotchas
- Linux truncates process names to 15 characters:
  `pgrep -x backstage_player` never matches. Use `pkill -f <full path>`.
  But a `-f` pattern also matches the shell running it when its own command
  line contains the pattern. Kill by PID, matching the end of the line
  (`ps -eo pid,args | awk '/[b]ackstage_tools$/ {print $1}'`), and keep the
  plain binary path out of that same command (no relaunch in it).
- wgpu's `vertex_attr_array!` packs attributes back to back, so padding in
  instance structs must come last. `item_attributes_match_the_struct_layout`
  guards this.
- wgpu 30 API changes:
  - `queue.present(texture)`
  - `get_current_texture()` returns a `CurrentSurfaceTexture` enum
  - `request_adapter` returns a `Result`
  - `VertexState.buffers` holds `Option`s
- Use `relm4::gtk` rather than depending on gtk4 separately under another
  name, so the versions stay unified.
