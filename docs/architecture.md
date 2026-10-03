# Architecture

## Layering

```
 tools process                           stage process (restartable)
┌──────────────────────┐    protocol     ┌──────────────────────────────┐
│ backstage_tools      │ ──Load───────>  │ backstage_stage              │
│  GTK 4 + Relm4       │   (base + log)  │  document (single writer)    │
│  timeline, layers,   │ ──Submit─────>  │  evaluate + renderer         │
│  library, properties │ ──Transport──>  │  editor view (hide/outline)  │
│                      │ ──Selection──>  │  mirrors of edit state       │
│  selection, playhead │ <──Loaded─────  │  on-stage edit hooks (M4):   │
│  document copy +     │ <──Committed──  │   selection, bounds, bezier, │
│  recovery log        │ <──frames─────  │   transform, snapping        │
└─────────┬────────────┘    (opt. B/C)   └─────────────┬────────────────┘
          │                                            │
          └──> backstage_core, backstage_protocol <────┘

 Test Movie: backstage_player process (runs scripts), started from the document
```

See [ADR 0002](adr/0002-stage-process-isolation.md) for how the processes
are split, and [ADR 0004](adr/0004-commands-and-document-authority.md) for
how edits flow between them.

**Built so far:**
- The process split, supervision, and crash/hang recovery.
- Shared-memory frames, and the stage rendering the evaluated project.
- The standalone player.
- Edit commands with undo/redo: the stage commits them, the editor keeps a
  copy and autosaves the log, and a restarted stage replays it (M2).
- The editor's panels on the real document (M3): a timeline with a
  playhead the editor owns and sends to the stage (`Transport`), Layers
  (the node tree, with editor-only hide/lock/outline), Properties, and a
  Library from which any composition can be entered. Open/Save, and
  restoring unsaved work after an editor crash.

**Still to come:** the on-stage edit hooks (M4).

**Where state lives** ([ADR 0005](adr/0005-edit-state.md)):
- **The document:** commits ordered by the stage, saved by the editor.
- **Edit state:** selection, playhead, the edited composition, and expanded
  nodes. Owned by the tools process; the stage keeps mirrors (`Transport`,
  `Selection`), resent on every connect.
- **Stage state:** hover and the drag in progress. Owned by the stage.

Rules:
1. `backstage_core` has **no** GPU, windowing, or GUI dependencies. It can be
   tested headless.
2. `backstage_render` draws the flat `Scene` that `backstage_core::evaluate`
   produces. It never walks the document tree or samples animations itself.
   It reads only the project settings it needs to frame the stage.
3. The editor and the player share the same core, render, and script crates.
   Authoring preview and published playback must behave the same.
4. The GUI framework only matters to the editor crate. Changing it should
   never touch core or render.
5. The tools process never links wgpu or the script VM. It only speaks the
   protocol. Runtime-side crates (core, protocol, render, script, stage,
   player) never depend on `backstage_tools` or on GTK/Relm4. Check with
   `cargo tree -p backstage_tools` and `cargo tree -p backstage_stage`.
6. The stage decides the order of edits. The tools process keeps a copy of
   every committed change, so a stage crash never loses committed work. It
   sends edits only to a stage whose replay matched its copy (ADR 0004).
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
- Nodes carry **editor-only flags** (hide, lock, outline) that playback
  never sees. The editor's stage applies them to the evaluated scene
  (`backstage_stage::view`); the player doesn't.
- Edits are **commands** (`backstage_core::Command`) whose `apply` returns
  the inverse. A **`Document`** wraps the project with its undo/redo history
  and a sequence number, and changes only through log entries (`Do`,
  `Undo`, `Redo`). See [ADR 0004](adr/0004-commands-and-document-authority.md).

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
