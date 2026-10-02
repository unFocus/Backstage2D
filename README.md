# Backstage2D

[![CI](https://github.com/unFocus/Backstage2D/actions/workflows/ci.yml/badge.svg)](https://github.com/unFocus/Backstage2D/actions/workflows/ci.yml)

A 2D animation and interactive-content engine in Rust. It starts from the
Flash Pro model (stage, timeline, reusable animated symbols) and modernizes it:
- Compositions with named, blendable animations.
- Time instead of frames.
- A pure evaluated scene instead of a mutable display list.
- A crash-isolated stage process.
- Vector rendering with lyon on wgpu.

Status: pre-alpha. M0–M3 are done: projects play live in the editor's stage
and in the standalone player; every edit is an undoable command that survives
stage and editor crashes; and the editor has a real timeline, Layers,
Properties, and Library, with Open/Save. M4 (on-stage editing) is next. See the [roadmap](docs/roadmap.md), and start with
[`docs/`](docs/README.md). The original 2012 AS3 Backstage2D is preserved on
the [`archive`](https://github.com/unFocus/Backstage2D/tree/archive) branch.

## Layout

| Path | Purpose |
|---|---|
| `crates/backstage_core` | Document model (projects, compositions, animations), time, project files, `evaluate`, and the mixer. No GPU or GUI dependencies. |
| `crates/backstage_render` | Draws an evaluated scene with wgpu (lyon meshes, gradients, MSAA). |
| `crates/backstage_script` | Placeholder for scripting (M6). |
| `crates/backstage_protocol` | Messages and shared-memory frames between stage and tools. |
| `crates/backstage_stage` | Editing engine process: renders the stage and streams frames. (runtime side) |
| `crates/backstage_player` | Standalone player: plays a project in its own window (winit + wgpu). |
| `crates/backstage_tools` | Editor GUI (GTK 4 + Relm4). Starts and supervises the stage. (editor side) |
| `samples/` | Sample projects. `bounce.bs2d` is also the file-format snapshot. |
| `scripts/check.sh` | Every check CI runs: fmt, clippy, tests, and headless smoke tests. |
| `docs/` | Vision, architecture, decision records, roadmap, dev setup, and testing. |


## Building

See [docs/dev-setup.md](docs/dev-setup.md) for the GTK 4 dev files.

```sh
export PKG_CONFIG=/home/linuxbrew/.linuxbrew/bin/pkgconf   # Homebrew GTK on Bazzite
cargo build --workspace
cargo run -p backstage_tools     # editor; starts backstage_stage itself
cargo run -p backstage_player -- samples/bounce.bs2d   # play a project in a window
cargo run -p backstage_core --example scene_dump -- samples/bounce.bs2d 0.5s   # print a scene
scripts/check.sh                 # everything CI checks (see docs/testing.md)
```

Editor keys: **Ctrl+O** / **Ctrl+S** / **Ctrl+Shift+S** open, save, and save
as; **Ctrl+Z** / **Ctrl+Shift+Z** undo and redo; **Enter** plays or pauses the
timeline; **Escape** clears the selection; **F2** renames the selected layer.
Double-click a composition in the Library (or an instance in Layers) to edit
it.

Player keys: **Space** pauses, **F** / **F11** toggles fullscreen, **Esc** or
**Ctrl+Q** quits. `--max-fps N` caps the frame rate (it otherwise follows the
display).

## License

Copyright (c) 2026 Kevin Newman. **All rights reserved.** The code is public
for reference only; no license is granted yet. See [LICENSE](LICENSE).
