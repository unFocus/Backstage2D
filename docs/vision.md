# Vision

## One-liner
A Flash Pro–style authoring tool and runtime for 2D vector animation and
interactive content, built in Rust on wgpu.

## What we keep from Flash
- **Stage + timeline** as the main way to author. Time (in seconds, not
  frames) is the primary axis.
- **Reusable animated units**: Flash's symbols (MovieClip, Graphic, Button)
  become **Compositions** with named animations that can be blended
  ([ADR 0003](adr/0003-document-model.md)).
- **A tree of objects** that users and scripts navigate. Underneath, it's
  evaluated from the document at each moment, not a mutable display list
  (ADR 0003).
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
