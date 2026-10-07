# Roadmap

The **document model** is the main thread: rendering, editing, undo, crash
recovery that keeps your work, and scripting all build on it. Each milestone
lands with tests in the tiers described in [testing.md](testing.md).

## Done

### M0: skeleton
- [x] Cargo workspace, crate boundaries
- [x] Platform policy ([platforms.md](platforms.md))
- [x] GUI framework: GTK 4 + Relm4 ([ADR 0001](adr/0001-gui-framework.md))
- [x] Stage/tools process split ([ADR 0002](adr/0002-stage-process-isolation.md))
- [x] Repo on GitHub (`unFocus/Backstage2D`, `main`). The original AS3 project is on the `archive` branch.
- [x] CI: `scripts/check.sh` on every push and pull request (Fedora 44 container, lavapipe, headless cage)

### M0.5: editor mockup
- [x] `backstage_protocol`: versioned messages, postcard framing, shared-memory frame ring
- [x] `backstage_render`: test scene rendered into a caller-provided texture view
- [x] `backstage_stage`: headless wgpu, 60 Hz frames into the ring, heartbeat
- [x] `backstage_tools`: GTK 4 + Relm4 window with library, stage, properties, and timeline sections
- [x] Stage supervisor: spawn, crash detection, hang watchdog, auto-restart, stale file cleanup
- [x] Pointer forwarding, and stage-side latency measured: median 16.6 ms on the RX 6800 (ADR 0002)
- [x] Automated tests: unit, integration, golden images, wire-format snapshot, dependency boundaries, headless UI smoke

### M1: first real content
The goal: a real project, animated over time and rendered with lyon, shown
both in the editor's stage and in a standalone player window. The model is
defined in [ADR 0003](adr/0003-document-model.md).
- [x] `Time` in flicks, typed IDs, and the `Project` / `Composition` / `Node` / `Animation` / `Track` / `Key` types, with validation
- [x] Project directory format (`name.bs2d/`, RON), with the sample project `samples/bounce.bs2d/` as the format snapshot
- [x] `evaluate` for a single animation: easing, loop modes, step, nested synced and free instances (`scene_dump` example prints a scene)
- [x] Mixer with crossfade blending (`scene_dump --crossfade squash@1s/0.2s` shows it)
- [x] lyon tessellation with a mesh cache. Draws the evaluated scene instead of the test scene. MSAA. Pixel-art mode.
- [x] Sample project (in the repo) and golden images of it
- [x] `backstage_player`: a winit + wgpu window that plays a project (Space pauses, F fullscreen)

### M2: edit commands and crash recovery with no lost work
The design is recorded in [ADR 0004](adr/0004-commands-and-document-authority.md).
- [x] `Command` type in core: apply returns the inverse, failures change nothing, results stay valid
- [x] Undo/redo log: `Document` (project, history, sequence number) changed only by `Entry`s (do, undo, redo), replay, and a document hash
- [x] Protocol: document snapshot on connect, edit commands in both directions
- [x] The stage is the only writer: it applies each command, numbers it, and sends it out
- [x] The editor keeps a copy of the document and autosaves the command log (restoring it after an editor crash came in M3)
- [x] On a stage restart, the editor sends the snapshot and replays the log. Test: replay gives an identical document (`a_restarted_stage_replays_the_editors_log`)
- [x] Undo/Redo and a temporary debug edit in the editor, exercised by the UI smoke test
- [x] ADR 0004: the command and document design
- [x] Bump `PROTOCOL_VERSION` (now 2)

### M3: editor panels on real data
- [x] Timeline widget: one animation at a time, a row per node, and an animation picker. Scrubbing moves the stage's playhead. Time grid snapping. It shows the edited composition, fits the animation to the panel width, and shows key marks per node without editing them. Protocol v3 adds `Transport`.
- Library, properties, and layers panels, in three steps:
  - [x] Selection and the Properties panel: the selected node's rest values (or the document settings when nothing is selected), edited through commands. Clicking a timeline row selects.
  - [x] Layers panel (outliner) owning the tree, rename, and hide/lock/outline (saved as editor-only flags, see the ADR 0003 addendum). The timeline mirrors its expand state and selection.
  - [x] Library panel, and entering a composition (double-click it in the Library, or an instance in Layers): the stage, timeline, layers, and properties switch to it, with a breadcrumb back. Compositions are edited in isolation, origin at the stage centre. Protocol v4.
- [x] Open, Save, and Save As (project folders), with a prompt for unsaved changes. Saving marks the recovery log as saved rather than clearing it, so undo still works after a save.
- [x] Offer to restore unsaved edits from a recovery directory after an editor crash. Undo history comes back too. Orphans with nothing unsaved are cleaned up.

## Next

### M4: on-stage editing
- [ ] Hit testing, selection, bounding boxes, transform handles, all drawn by the engine. Spatial grid snapping.
- [ ] Drag gestures become commands. Undo works across them.
- [x] Render as soon as input arrives instead of on the fixed tick; idle while paused. Protocol v6 carries pointer presses, releases, and modifiers. The latency probe's median went from 16.6 ms to 0.9 ms (ADR 0002).

### M5: Test Movie
- [ ] Start `backstage_player` from the current document (Flash's Ctrl+Enter)

### M6: scripting
- [ ] The engine API scripts will call ([scripting.md](scripting.md))
- [ ] Prototype binding (Rhai or Luau)
- [ ] Scripts and events (markers, input), running in the player only
- [ ] State machines and blend spaces (Rive-style), if not needed earlier

## Later
- **Deferred from M3:**
  - Timeline: zoom and scrolling, one row per property, and editing keys
  - Layers: reordering and grouping by drag (`MoveNode` exists)
  - Editing a composition in place, with its parent shown around it, and
    returning to where you were when you leave it
  - Library: renaming compositions, opening assets
  - Restoring a choice of several crashed sessions, not just the newest
- Zero-copy stage frames (DMA-BUF import with `GdkDmabufTexture` and `GtkGraphicsOffload`)
- Dockable panels (libpanel)
- Drawing tools, shape tweens, masks, text, audio, filters, blend modes
- Custom scripting language
- macOS and Windows 11 editor; web player
