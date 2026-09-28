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

## Next

### M1: first real content *(next)*
The goal: a real document, animated by the timeline and rendered with lyon,
shown both in the editor's stage and in a standalone player window.
- [ ] `backstage_core` document model: `Document`, `Symbol` (MovieClip / Graphic / Button), `Timeline`, `Layer`, `Keyframe`, `Instance`, shapes (paths, fills, strokes)
- [ ] Text document format (RON) with insta snapshot tests
- [ ] Timeline evaluation: a pure function from document + frame to a list of what to draw (transforms, color transforms, depth)
- [ ] Motion tweens with easing, and nested MovieClip playheads
- [ ] lyon tessellation with a mesh cache. Draws the evaluated list instead of the test scene. MSAA.
- [ ] Golden images of a sample document
- [ ] `backstage_player`: winit + wgpu window that plays a document file

### M2: edit commands and crash recovery with no lost work
- [ ] `Command` type in core with apply and undo. Undo/redo log.
- [ ] Protocol: document snapshot on connect, edit commands in both directions
- [ ] The stage is the only writer: it applies each command, numbers it, and sends it out
- [ ] The editor keeps a copy of the document and autosaves the command log
- [ ] On a stage restart, the editor sends the snapshot and replays the log. Test: replay gives an identical document.
- [ ] Bump `PROTOCOL_VERSION`

### M3: editor panels on real data
- [ ] Timeline widget built from the editor's copy of the document; scrubbing moves the stage's playhead
- [ ] Library, properties, and layers panels
- [ ] Open and save files

### M4: on-stage editing
- [ ] Hit testing, selection, bounding boxes, transform handles, all drawn by the engine
- [ ] Drag gestures become commands. Undo works across them.
- [ ] Render as soon as input arrives instead of on the fixed tick. Measure with the latency probe.

### M5: Test Movie
- [ ] Start `backstage_player` from the current document (Flash's Ctrl+Enter)

### M6: scripting
- [ ] The engine API scripts will call ([scripting.md](scripting.md))
- [ ] Prototype binding (Rhai or Luau)
- [ ] Frame scripts and events, running in the player only

## Later
- Zero-copy stage frames (DMA-BUF import with `GdkDmabufTexture` and `GtkGraphicsOffload`)
- Dockable panels (libpanel)
- Drawing tools, shape tweens, masks, text, audio, filters, blend modes
- Custom scripting language
- macOS and Windows 11 editor; web player
