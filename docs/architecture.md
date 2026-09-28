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

## Core data model
Defined in [ADR 0003](adr/0003-document-model.md):
- A **Project** holds a library of **Compositions** and assets.
- Each composition has a fixed **node tree** and named **animations** that
  key node properties over time. Times are in flicks, not frames.
- At runtime, `evaluate(&Project, &RuntimeState, now) -> Scene` produces a
  flat list of draw items. The renderer, hit testing, and the editor's
  on-stage tools all consume it.
- The runtime state (mixers, free-running clocks, script overrides) is small,
  data-oriented, and serializable.

## Frame loop (player / stage)
Runs at the display's refresh rate (optionally capped). Nothing in the
document depends on it.

1. Handle input and dispatch events.
2. Advance the clocks by the real elapsed time: free-running instances,
   mixers, and crossfades.
3. Run scripts (M6). They write overrides and runtime state, never the
   document.
4. `evaluate` the scene at the new time: flattened transforms, culling.
5. Submit to wgpu.

Game logic may later run on a fixed tick (for example 30 or 60 Hz) with
rendering interpolated in between. That choice belongs to scripting (M6).
Animation playback is already exact, because it is sampled from time.
