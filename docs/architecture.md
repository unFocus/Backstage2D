# Architecture

## Layering

```
 tools process                           stage process (restartable)
┌──────────────────────┐    protocol     ┌──────────────────────────────┐
│ backstage_tools      │ ──commands────> │ backstage_stage              │
│  GUI toolkit (TBD)   │ ──queries─────> │  document (edit authority)   │
│  timeline, library,  │ <──committed─── │  display list + renderer     │
│  properties panels   │    commands     │  on-stage edit hooks:        │
│  document copy +     │ <──frames────── │   selection, bounds, bezier, │
│  autosave journal    │    (opt. B/C)   │   transform, snapping        │
└─────────┬────────────┘                 └─────────────┬────────────────┘
          │                                            │
          └──> backstage_core, backstage_protocol <────┘

 Test Movie: backstage_player process (runs scripts), started from the document
```

See [ADR 0002](adr/0002-stage-process-isolation.md) for how the processes
are split and how state is copied between them.

Rules:
1. `backstage_core` has **no** GPU, windowing, or GUI dependencies. It can be
   tested headless.
2. `backstage_render` receives a *flattened render list* built from the
   display list. It never walks the document model directly.
3. The editor and the player share the same core, render, and script crates.
   Authoring preview and published playback must behave the same.
4. The GUI framework only matters to the editor crate. Changing it should
   never touch core or render.
5. The tools process never links wgpu or the script VM. It only speaks the
   protocol. Runtime-side crates (core, protocol, render, script, stage,
   player) never depend on `backstage_tools` or on GTK/Relm4. Check with
   `cargo tree -p backstage_tools` and `cargo tree -p backstage_stage`.
6. The stage decides the order of edits. The tools process keeps a copy of
   every committed change, so a stage crash never loses committed work.
7. User scripts never run in the editing stage. They run in the player
   process (Test Movie).

## Core data model (sketch)

- `Document`: the library (symbols, bitmaps, sounds, fonts), root timeline,
  stage size, frame rate.
- `Symbol`: `MovieClip | Graphic | Button`. Each owns a `Timeline`.
- `Timeline`: ordered `Layer`s. Each layer holds `Keyframe` spans.
- `Keyframe`: a list of placed `Instance`s, plus tweens to the next keyframe.
- `Instance`: a symbol reference, transform, color transform, filters, blend
  mode, and an optional name for scripts.
- `DisplayObject` (runtime): an instance that is alive on stage, with
  playhead state for its nested timeline.

The *document* (authored and immutable during playback) is kept separate from
the *runtime display list* (mutable, script-owned). Flash mixed the two, and
that caused many of its bugs.

## Frame loop (player / stage)

1. Handle input and dispatch events.
2. Run scripts (enter-frame, frame scripts).
3. Advance playheads and apply timeline changes to the display list.
4. Build the render list (flatten transforms, cull).
5. Submit to wgpu.

Open question: fixed-rate timeline ticks with interpolated rendering, or
variable rate? Leaning fixed-rate for determinism.
