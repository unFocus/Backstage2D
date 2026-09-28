# Compositions, animations, and the scene

How the model in [ADR 0003](adr/0003-document-model.md) behaves. This
replaces the original Flash-style notes: there are no frames, no keyframe
spans, and no mutable display list.

## Compositions
- A **Composition** is a fixed **node tree** plus named **animations**. The
  stage is the root composition.
- Nodes are groups, shapes, flipbooks, bitmaps, nested composition
  instances, and masks. Sibling order is z-order.
- **Top-level groups are shown as layers** in the editor (lock, hide,
  outline), as in Flash.

## Animations
- An animation keys node properties over **time** (flicks). A 2 s tween is
  2 s at any display rate.
- Easing applies per key: hold, linear, cubic bezier, and named presets.
- **Loop modes:** once, loop, and ping-pong.
- **Markers** are named times, used for events and script cues.
- **Optional `step`** holds values for a fixed interval, for the "on twos"
  or retro look. The stored keys stay continuous.
- **Frame-by-frame** animation uses a flipbook node: its `drawing` property
  is keyed with hold keys.
- **Appearing and disappearing** is keyed visibility.

## Nesting and blending
- A nested instance is either:
  - **Synced:** it plays one of its animations on the parent's clock, with
    an offset and repeat. This is the Flash "Graphic" trick.
  - **Free:** it has its own clock and its own **mixer**.
- The **mixer** blends several animations by weight:
  - Numeric properties interpolate. Rotation takes the shortest arc; colors
    blend in linear space.
  - Discrete properties take the value from the strongest animation.
- **Crossfade** comes first. Blend spaces, additive layers, and state
  machines follow.

## The scene
- `evaluate(project, runtime_state, now)` produces the scene to draw. It's a
  pure function, so scrubbing, seeking, and rewinding are free.
- Each property is resolved in order: **script override > blended animation
  > rest value.** The timeline and scripts never fight over a shared object.
- Scripts see a tree of objects addressed by **instance path** (the chain of
  instance node IDs). These are handles into the runtime state, not mutable
  objects. Objects created by scripts live in the runtime state, under a
  parent path, until the script removes them.

## Editing aids (not playback)
- **Time grid:** keys snap to N ticks per second (default 60; 30 for classic
  games).
- **Spatial grid:** objects snap to a pixel grid on the stage.
- **Pixel-art mode** (project setting): whole-pixel positions and
  nearest-neighbor bitmaps.

## Open questions
- Event model for input: capture/bubble like the DOM, or simpler routing
  through state machines?
- Hit testing: shape-accurate or bounds-only by default?
- Masks: exact clipping semantics and nesting limits.
