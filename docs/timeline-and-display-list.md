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
  - **Synced:** it plays one of its animations on its parent's clock, with
    an offset and repeat. This is the Flash "Graphic" trick.
  - **Free:** it has its own clock and its own **mixer**.
- A **mixer** holds weighted **layers**. Each layer is an animation with
  its own start time, and a weight that is either fixed or fading linearly.
  An instance without a mixer plays its configured (or default) animation
  at full weight from time zero.
- **Runtime API** (`RuntimeState`):
  - `play` switches immediately.
  - `crossfade` fades every other layer out and the target in. A layer
    that's still fading comes back without restarting, so quickly moving on
    and off a button is smooth.
  - `prune` drops layers that have finished fading out.
- **Blending rules**, for each property keyed by at least one layer:
  - **Numbers, colors (`Rgba`), and times.** With total weight `W`:
    - If `W ≥ 1`, the weighted average.
    - If `W < 1`, `rest·(1−W) + Σ wᵢvᵢ`: the remainder falls back to the
      rest value.

    So a crossfade is exactly `(1−f)·A + f·B`. A property only A animates
    eases back to rest as B takes over.
  - **Rotation and skew** take the shortest arc (350° and 10° meet at 0°).
  - **Discrete** properties (visibility, blend mode, flipbook drawing)
    come from the strongest layer. A later layer wins a tie.
  - **Synced children** follow the parent's **dominant** (highest-weight)
    layer's time.
- **Later:** blend spaces, additive layers, eased fades, and state machines.

## Evaluation rules
Implemented in `backstage_core::eval`.
- **Clocks:**
  - The root composition and **free** instances run on the global clock,
    minus their player's start time (0 if there's no player).
  - **Synced** instances run on their **parent's local animation time**
    plus `offset`. The parent's time is already looped or ping-ponged, so
    nested loops line up with the parent's loop, as Flash Graphics did.
- **Local time:** the loop mode (or the instance's `repeat`) maps the clock
  into `0..=duration`. Then `step` quantizes it. All of this is exact
  integer math on flicks.
- **Keys:** before the first key, the first value applies; after the last,
  the last. In between, the earlier key's ease applies. Discrete
  properties hold.
- **Transform order:** `T(position) · R(rotation) · Skew · S(scale) ·
  T(−pivot)`. Angles are degrees, and positive rotation is clockwise
  (y is down). The world transform is `parent · local`.
- **Inheritance:**
  - Opacity multiplies down the tree.
  - Color transforms apply the child's first, then the parent's.
  - An invisible node hides its whole subtree.
- **Painter's order:** depth-first, in child order. An instance's content
  draws before the instance node's own children.
- **Not yet:** masks (skipped for now) and script overrides.

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
- **Time grid:** keys and the timeline's playhead snap to N ticks per
  second (default 60; 30 for classic games). The timeline's Snap toggle and
  grid menu change `EditorPrefs`, which is an undoable edit saved with the
  project.
- **The editor's playhead:** the editor owns a transport (the animation
  the root composition plays, its clock, and whether it's playing) and
  sends it to the stage (`ToStage::Transport`), again whenever a stage
  connects. So a restarted stage comes back at the same moment. The stage
  starts paused at 0, as in Flash. Enter plays and pauses, and scrubbing
  pauses. Seeking just evaluates another moment (`evaluate` is pure), so
  free-running nested instances move with it. Without a `Transport`, the
  stage (and the player) plays the default animation from zero.
- **Spatial grid:** objects snap to a pixel grid on the stage.
- **Pixel-art mode** (project setting): whole-pixel positions and
  nearest-neighbor bitmaps.

## Open questions
- Event model for input: capture/bubble like the DOM, or simpler routing
  through state machines?
- Hit testing: shape-accurate or bounds-only by default?
- Masks: exact clipping semantics and nesting limits.
