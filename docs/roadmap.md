# Roadmap

## M0: skeleton *(current)*
- [x] Cargo workspace, crate boundaries
- [ ] Planning docs reviewed
- [x] Platform policy ([platforms.md](platforms.md))
- [ ] Stage process isolation ([ADR 0002](adr/0002-stage-process-isolation.md))
- [x] GUI framework decision: GTK 4 + Relm4 ([ADR 0001](adr/0001-gui-framework.md))

## M0.5: Editor mockup *(done)*
- [x] `backstage_protocol`: versioned messages, postcard framing, shared-memory frame ring
- [x] `backstage_render`: test scene rendered into a caller-provided texture view
- [x] `backstage_stage`: headless wgpu, 60 Hz frames into the ring, heartbeat
- [x] `backstage_tools`: GTK 4 + Relm4 window with library, stage, properties, and timeline sections
- [x] Stage supervisor: spawn, crash detection, hang watchdog, auto-restart, stale file cleanup
- [x] Pointer forwarding (crosshair drawn by the stage)
- [x] Measure stage-side latency of option B: median 16.6 ms pointer → published frame on the RX 6800 (see ADR 0002)
- [x] Automated tests: unit, integration, golden images, wire-format snapshot, dependency boundaries, headless UI smoke ([testing.md](testing.md))

## M1: something on screen
- [ ] winit + wgpu window in `backstage_player`
- [ ] Core types: `Document`, `Symbol`, `Timeline`, `Layer`, `Keyframe`, `Instance`
- [ ] Hard-coded document with shapes on a timeline, played at a fixed rate
- [ ] lyon tessellation + instanced draw

## M2: timeline playback
- [ ] Nested MovieClips, motion tweens with easing
- [ ] Document serialization (RON or similar text format)
- [ ] Masks

## M2.5: stage process
- [ ] `backstage_protocol` crate: versioned message types
- [ ] Stage process: document authority, command apply + broadcast, undo log
- [ ] Stage window (ADR 0002 option A), with `xdg-foreign` tie to the tools window
- [ ] Tools-side mirror + autosave journal; supervisor: spawn, heartbeat, restart + replay
- [ ] On-stage edit hooks: selection, bounding box, transform handles
- [ ] Shared-memory frame streaming (option B) + drag-latency measurement

## M3: editor shell
- [ ] `backstage_tools` on the chosen GUI framework
- [ ] Timeline, library, properties, and layers panels driven by the document copy
- [ ] Test Movie: spawn `backstage_player` from the current document

## M4: scripting
- [ ] Host API
- [ ] Prototype language binding
- [ ] Frame scripts and events

## Later
Drawing tools, shape tweens, text, audio, filters, web export, and the custom
language.
