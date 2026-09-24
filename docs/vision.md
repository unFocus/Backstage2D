# Vision

## One-liner
A Flash Pro–style authoring tool and runtime for 2D vector animation and
interactive content, built in Rust on wgpu.

## What we keep from Flash
- **Stage + timeline** as the main way to author. Time is the primary axis.
- **Symbols**: reusable MovieClips, Graphics, and Buttons, each with its own
  nested timeline.
- **Display list**: a retained tree of display objects. Scripts and the
  timeline both change it.
- **Vector-first art**: shapes, strokes, gradients, and tweens between them.
- **A standalone player**: published content runs without the editor.

## What we modernize
- **Data-oriented core.** The document is plain data that serializes to a
  diffable text format (for git) and a compact binary format (for publishing).
- **Non-destructive, curve-based animation.** Keyframes plus easing curves and
  a graph editor, not frame-by-frame tweens baked into the file.
- **Undo/redo as a first-class command log.** It also opens the door to
  collaboration later.
- **Hot reload** of scripts and assets while the player is running.
- **GPU rendering** of vectors: tessellation first, compute-based rasterization
  later.
- **A small, typed scripting language** with fast iteration and good error
  messages, instead of AS2/AS3 baggage.
- **Web target** through wgpu's WebGPU/WebGL backends.

## Non-goals (for now)
- Opening SWF or FLA files. Maybe later as an importer, never as a
  compatibility promise.
- 3D.
- Competing with game engines on ECS or physics scale.
