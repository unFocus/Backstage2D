# Testing

Run everything with:

```sh
scripts/check.sh
```

It runs `cargo fmt --check`, clippy with `-D warnings`, `cargo test
--workspace`, and the two headless smoke tests (the editor and the player).
It takes about 1.5 minutes on the dev host with a warm build cache. Tests need **no GPU and no display**:
- Rendering uses wgpu's software fallback adapter (Mesa **lavapipe**) through
  `BACKSTAGE_WGPU_FALLBACK=1`.
- The UI runs inside a **headless `cage`** Wayland compositor.
- **Real input** comes from `backstage_uidriver`, a test-only Wayland
  client. It drives cage's virtual pointer and keyboard
  (`zwlr_virtual_pointer_v1`, `zwp_virtual_keyboard_v1`) and captures the
  screen (`zwlr_screencopy_v1`). GTK sees real input, so the path from a
  press to a `Pointer` message is covered too. See "Headless sessions"
  below.

## Tiers

| Tier | Where | What it covers |
|---|---|---|
| Unit | `#[cfg(test)]` in each crate | **Core:** exact time strings and grids, IDs, colors, validation (one test per error), project I/O, edit commands (each command's undo and redo, each error leaving the project untouched, and a proptest of random edit sequences that must stay valid and undo exactly), the document's undo/redo history (a proptest checks that replaying the log gives an identical document and hash, and that every entry survives the log's one-line RON), easing (vs. a brute-force reference), clocks, track sampling, blending, the mixer, and `evaluate` on the sample. **Render:** tessellation, paints, framing, the vertex layout, and hit testing (fills inside their outline only, strokes within half their width plus slop, transformed items, tight bounds that include strokes, items scaled flat). **Protocol:** `PickMode::apply` (replace, add without duplicates, toggle, the last picked as primary), round-trips (proptest, including document entries as RON text), garbage never panics, project snapshots, the message size limit, why core types can't go through postcard directly, and the seqlock stress test. **Stage:** the editor's view of a scene (hidden nodes, groups, and instances leave with their content; outlined items get their node's colour; no flags change nothing; another composition's own flags and its shift to the origin), the transport's clock (paused, playing, never backwards) and the animation it puts on the shown composition. **UI driver:** command parsing, and screen pixels to RGBA. **Tools:** the smoke test's stage clicks (their order, each expected selection, and click targets landing inside the named node), the `StageHealth` crash/hang policy, and the `WorkingCopy` (commits in sequence order, out-of-sync commits changing nothing, the `Load` it sends, hash checks, the recovery log on disk, including a line cut off by a crash, commits ignored between a `Load` and its `Loaded`, Save writing the project and marking the log in `meta.ron`, dirty tracking through undo, which recovery directories a clean exit removes, `save_target`'s rules for Save As, and restoring a crashed editor's directory with its history and saved state, cutting off a partial last line before appending). The recovery scan: locked directories skipped, unreadable ones kept, ones with nothing unsaved deleted, candidates newest first. The timeline: rows in tree order with depths and each node's key times, the animation fallback, x/time mapping, scrubbing (snapped to the grid, unsnapped, clamped), grid-line thinning, and the playhead (pause, play, ping-pong like the stage's clock, a play-once animation starting over). The properties panel: document vs. node models, the fields per node kind (Drawing only on flipbooks), what the shown animation keys, a locked node greying out Properties, the edit helpers (one-property `SetRest` that undoes exactly, renames, settings, and `None` for no-op edits so widget refreshes never submit). `Props::get`/`set` round trips for every property. Timeline presses: select a row, expand or collapse, clear, or scrub. The Library (compositions then assets, sorted, the stage marked; only compositions can be edited). The breadcrumb path (entering, going back, entering one already on the path, pruning missing compositions, crumb names), and any composition's timeline, animations, and playhead. Layers' flag toggles (`flags_edit` turns one flag on and off and undoes exactly). The node tree as rows (`visible_rows`: depths, expanders only where there are children, collapsed groups, stale IDs), and the timeline mirroring it with a collapsed row summarizing its subtree's keys. The selection sync (sends a change once, again after a stage connects, and on a change of nodes, order, or composition); the stage's selection mirror (only the shown composition's existing nodes, in order). Stage picking: top-level nodes, instances picked whole even with Ctrl, groups whole unless Ctrl, locked nodes letting clicks through (also through a locked group), and bounds over a node's subtree. The editor applies picks only for the composition it edits. Argument parsing in every binary. |
| Integration | `crates/backstage_stage/tests/`, `crates/backstage_tools/tests/supervisor.rs` | The real stage binary driven over the real protocol (handshake, frames, resize, shutdown, disconnect, version mismatch, pointer, a paused stage drawing nothing until input arrives and then drawing it in one frame, the stage reporting its framing in logical pixels (the same at scale 2, again when the stage size changes), clicks picking, toggling, and clearing the selection (and leaving it alone when the node is already selected), clicks passing through a locked node, and selection and hover boxes drawn by the stage before the editor echoes the selection, and the document: no frames before `Load`, log replay and its hash, committed edits with undo and redo showing on stage, rejected entries, and a bad snapshot; the transport pausing, seeking, choosing the animation, and playing; a node hidden with `SetNodeFlags` leaving the stage and coming back on undo; another composition shown on its own around the stage centre, and an unknown one falling back to the root). The real `Supervisor` spawning, killing, restarting, and cleaning up stages, and M2's acceptance test: edits committed through the editor's `WorkingCopy` survive a stage kill, the restarted stage replays them into an identical document (matching hash), and undo history carries over. A second `Load` on the same stage (File → Open) replaces its document, and a commit for the old document still in flight is ignored. A copy restored from a crashed editor's recovery directory loads onto a fresh stage with a matching hash, and redo works there. |
| Regression: golden images | `crates/backstage_render/tests/golden.rs`, `tests/golden/*.png` | The sample project rendered on lavapipe at fixed times (lyon meshes, gradients, strokes, MSAA, the crosshair), the editor's outline view, and selection and hover boxes |
| Regression: project format | `samples/bounce.bs2d/` + `sample_matches_checked_in_files` in `backstage_core` | The on-disk project format, byte for byte |
| Regression: wire format | `crates/backstage_protocol/src/snapshots/` (insta) | Byte encoding of every protocol message |
| Regression: architecture | `crates/backstage_tools/tests/dependency_boundaries.rs` | Runtime crates never link GTK/Relm4. The tools crate never links wgpu, lyon, the renderer, or the VM. The core crate stays free of GPU/GUI crates. Nothing ships the UI test driver. |
| Player smoke | `crates/backstage_player/tests/player_smoke.rs` (ignored by default) | The player opens a Wayland window in headless cage and presents 30 frames |
| UI smoke | `crates/backstage_tools/tests/ui_smoke.rs` (ignored by default) | The real editor in headless cage under `backstage_uidriver`, driven by `src/smoke.rs` through the same paths as the user: frames arrive; **real clicks on the stage** select the ground, Shift-click adds a ball, and a click on empty stage clears the selection, each checked against the editor's selection, with screenshots kept in `target/tmp/ui-smoke-shots/`; the timeline is scrubbed, the first node is selected (through the Layers panel), the first instance's composition is entered (through the Library) and left, a nudge is committed, the stage is killed, the restarted stage replays the log into a matching document, the project is saved (to a temporary copy of the sample, checked on disk afterwards), and an undo commits on it. The run exits without cleaning up, like a crash, and a second run (`BACKSTAGE_SMOKE=restore`) restores the unsaved undo and checks that the stage loaded it with its history. |
| Probe | `latency_probe` in `stage_protocol.rs` (ignored) | Pointer → published frame latency on a paused stage (prints numbers, asserts nothing) |

## Updating reference data
- **Golden images:** when a rendering change is intended, look at the
  failure output in `target/golden-failures/` (`*.actual.png`,
  `*.diff.png`, with magenta marking bad pixels), then re-bless:
  ```sh
  BACKSTAGE_BLESS=1 cargo test -p backstage_render --test golden
  ```
  At most 16 pixels may differ by more than 2 per channel. The budget is
  small on purpose: a 1 px line moving changes only about 100 pixels.
  Goldens are tied to lavapipe, so after a Mesa upgrade, review the diffs
  and re-bless.
- **Project format (sample project):** `samples/bounce.bs2d/` is
  `backstage_core::sample::bounce()` saved to disk. When a format change is
  intended, re-bless it and review the diff of the `.ron` files:
  ```sh
  BACKSTAGE_BLESS=1 cargo test -p backstage_core sample_matches
  ```
  A format change that breaks old files also needs a `FORMAT_VERSION` bump.
- **Wire-format snapshot:** a diff here means old and new processes can no
  longer talk. Bump `PROTOCOL_VERSION`, then accept the new snapshot:
  ```sh
  INSTA_UPDATE=always cargo test -p backstage_protocol wire_format
  ```
  `cargo install cargo-insta` adds an interactive `cargo insta review`.

## Useful single runs
```sh
cargo test -p backstage_protocol                     # fast, pure
cargo test -p backstage_stage                        # real stage process
cargo test -p backstage_tools --test ui_smoke -- --ignored
cargo test -p backstage_player --test player_smoke -- --ignored
cargo test -p backstage_stage --test stage_protocol -- --ignored --nocapture latency
```
The latency probe uses the real GPU unless `BACKSTAGE_WGPU_FALLBACK=1` is
set.

A paused stage only draws when something changes. A stage test that waits
for a frame has to change something first. The `transport`/`show` helpers
end with a pointer `Leave` for that reason.

## Headless sessions
`backstage_uidriver` runs an app in a headless compositor and feeds it
commands. That's how to look at interactive states (hover, selection,
drags) without touching the desktop:
```sh
cargo build -p backstage_tools -p backstage_stage -p backstage_uidriver
cat > /tmp/s.txt <<'END'
wait 3000                 # let the editor start
move 625 510              # output pixels; the output is 1280 × 720
shot /tmp/hover.png
click 750 259 shift       # also: ctrl, alt
shot /tmp/clicked.png
END
env -u WAYLAND_DISPLAY WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
  GSK_RENDERER=cairo BACKSTAGE_WGPU_FALLBACK=1 \
  cage -- target/debug/backstage_uidriver --script /tmp/s.txt -- target/debug/backstage_tools
```
- **Commands:** `move X Y`, `down`, `up`, `click X Y [shift] [ctrl] [alt]`,
  `key NAME down|up`, `shot PATH`, and `wait MS`.
- **From the app:** without `--script`, the app gets a socket in
  `BACKSTAGE_UI_DRIVER` and sends the same commands itself. The smoke test
  does this. It finds its targets from `ToTools::Framing`, which tells the
  editor where the stage sits.
- **Xwayland noise:** cage starts Xwayland, which prints xkbcomp warnings
  about the virtual keyboard's keymap. They're harmless.

## Requirements
- Mesa lavapipe (`lvp_icd`). It ships with the system Mesa on Bazzite.
- `cage` for the UI smoke test (system package on Bazzite, and in CI's
  Fedora image). The test skips itself if `cage` is missing. The UI driver
  needs no system libraries: its Wayland client is pure Rust.
- GTK 4 dev files, as described in [dev-setup.md](dev-setup.md).

## CI
`.github/workflows/ci.yml` runs `scripts/check.sh` on every push to `main`,
on pull requests, and on demand.
- **Environment:** a `fedora:44` container, to match the dev host's GTK,
  cage, and Mesa lavapipe.
- **On failure:** golden-image diffs are uploaded as the `golden-failures`
  artifact.
- **If lavapipe output differs between CI and local:** re-bless from CI's
  `*.actual.png` files, after reviewing them.
