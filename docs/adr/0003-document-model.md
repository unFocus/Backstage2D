# ADR 0003: Document model, scene model, and time

**Status:** Accepted (2026-09-28)

## Context
M1 needs a document model in `backstage_core`. Requirements from the user:
- Flash is the starting point, not a constraint. Better names and better
  structures are welcome.
- **Time, not frames.** A 2-second tween takes 2 seconds at any frame rate.
- Compare Flash's display list with the alternatives before committing.

Constraints from earlier ADRs:
- The document is plain, serializable data that can be edited through
  commands (ADR 0002).
- The stage process evaluates and renders it. The tools process keeps a
  copy of it.

## How others model this

| Engine / tool | Reusable unit | Structure | Animation | Time |
|---|---|---|---|---|
| **Flash** | Symbol (MovieClip / Graphic / Button) | Mutable display list of objects | Layers of keyframe spans. A keyframe can swap content. Tweens between keyframes. | Frames at a document fps |
| **After Effects / Lottie** | Composition (nested as "precomp") | Layers with in/out times | Keyframed properties on layers, bezier easing | Frames at comp fps, continuous interpolation |
| **Rive** | Artboard (nested artboards) | Fixed component tree | Named animations key properties. State machines drive interactivity. | Frames at a per-animation fps |
| **Spine / DragonBones** | Skeleton | Fixed bone tree | Named animations: per-property curves | **Seconds** |
| **Godot** | Scene (instanced) | Node tree | `AnimationPlayer` tracks key any property path | **Seconds** |
| **Unity / Bevy** | Prefab / scene | GameObjects + components / ECS | Clips of curves | Seconds |

Two families stand out:
- **Content over time (Flash, After Effects).** What exists changes over
  time: spans start and end, and frame-by-frame drawings swap. This is the
  natural fit for animators.
- **Fixed tree with keyed properties (Rive, Spine, Godot).** The structure
  never changes; animations only move values. This is the natural fit for
  rigs, UI, and games.

We want both: cel-style content changes *and* property animation. We also
want **blending between animations** (for example, a crossfade from walk to
run), which the user relied on in Flash by nesting animations. Blending only
works when every animation targets the *same* objects. So we take the
**fixed tree** family:
- A composition has a fixed tree of nodes.
- Its named animations key those nodes' properties.
- Cel animation is a *flipbook* node whose current drawing is a keyed
  property.
- Appearing and disappearing are keyed visibility.

## Display list vs the alternatives

| Model | Strengths | Weaknesses for us |
|---|---|---|
| **Flash display list**: a tree of mutable objects shared by the timeline and scripts | Intuitive. Hierarchical transforms. Event bubbling. | The timeline and scripts fight over the same objects (Flash's worst bugs). Runtime state is scattered across objects, so it's hard to serialize, diff, or send to another process. Inheritance-heavy. |
| **Scene graph + components** (Godot, Unity) | Composition over inheritance. Familiar to game developers. | Still one mutable tree that both animation and code write to. |
| **ECS** (Bevy) | Very fast. Data-oriented. | Hierarchy and authoring are awkward. Overkill for a document that people edit. |
| **Evaluated scene**: the document plus a small runtime state go through a pure function to produce the scene to draw (React-like; also how Rive and Spine apply animations) | Deterministic. Easy to test. Any time can be evaluated (scrubbing, seeking, rewinding). The editor, player, and hit testing share one function. No timeline-vs-script fights: each property has a clear order of precedence. | Scripts need a stable API for "objects" even though those objects are derived each frame. |

**Proposal: the evaluated-scene model, presented to users and scripts as a
tree.** There are three layers:

1. **Document.** Authored, immutable during playback, serializable, and
   edited only through commands (M2).
2. **Runtime state.** Small and data-oriented. It holds free-running
   playheads, active states, script overrides, and objects created by
   scripts, all in arenas keyed by stable IDs. It serializes, so a running
   movie can be saved, rewound, or moved to another process.
3. **Evaluated scene.** Derived every frame by
   `evaluate(&Document, &RuntimeState, time) -> Scene`. It's a flat list of
   what to draw (world transform, color transform, depth, mesh or bitmap).
   The renderer draws it, and hit testing and the editor's on-stage tools
   read it.

The "display list" still exists as a concept, as the tree users see in the
editor and scripts navigate. But it's a *view* over the document and runtime
state, not a pile of mutable objects.

Each property is resolved in this order: **script override > animation >
rest value**. This is how Flash's "timeline and script disagree" problem gets
settled.

## Time
- **All times are `Time(i64)` in flicks**: 705,600,000 per second. That
  divides exactly into every common frame rate (24, 25, 30, 48, 50, 60, 90,
  100, 120, and more) and audio rate. So keys snap to any frame grid exactly,
  with no floating-point drift, and 64 bits cover about 400 years.
  Interpolation converts to `f64` only to compute the progress fraction.
- **Durations are real time.** A 2 s tween takes 2 s at any display rate.
- **The project has no frame rate.**
  - **Editor:** the timeline ruler shows time (seconds, with ticks that adapt
    to the zoom level). Keys snap to a configurable *time* grid, set as ticks
    per second. The default is **60**; classic-game projects typically use
    30. It's stored in the project and can be turned off. The time grid only
    controls where keys land, not how fast anything plays.
  - **Runtime:** renders at the display's refresh rate (vsync), and adapts
    to the machine. The player can cap it (`max_fps`) to save power.
- **Optional stepped playback per animation** (`step: Option<Time>`, like
  After Effects' "posterize time" or animating "on twos"). A stylistic
  choice to hold values for, say, 1/12 s. Off by default. Stepping is
  applied at evaluation time; the stored keys stay continuous.
- **Scripts** receive `dt` in seconds. The fixed-step option for game logic
  is decided in M6.

## Proposed model

### Vocabulary (Flash equivalent in parentheses)
- **Project**: the document. Holds the library of compositions and assets,
  the root composition, and settings:
  - stage size (configurable, e.g. 320×180 for pixel art)
  - background
  - **pixel-art mode**: render positions snapped to whole pixels, and
    nearest-neighbor bitmap sampling
- **Composition** (Symbol / MovieClip / Graphic / Button): a reusable unit.
  It has a **node tree** (its content in rest pose) and named
  **animations**. The stage is simply the root composition.
- **Node**: an element in a composition's tree. It has a stable `Id`, a
  name, *rest* properties, and children. Sibling order is z-order. Kinds:
  - `Group`: a container. Also covers Flash layers and layer folders.
  - `Shape`: vector paths, fills, and strokes.
  - `Flipbook`: a list of drawings plus an animatable `drawing` index. This is
    how frame-by-frame / cel animation works. Drawings are vector shapes
    stored **inline** in the composition file, each with its own
    `DrawingId`, because they belong to that composition. Bitmaps are always
    assets. Drawings can move to `assets/` later if files grow; the IDs make
    that painless.
  - `Bitmap`: an image asset.
  - `Instance`: a nested composition. See *time modes* below.
  - `Mask`: its children clip the next siblings (details in M2).
  - `Text`: later.
- **Animation** (the timeline): a name, a duration, a loop mode (`Once`,
  `Loop`, or `PingPong`), an optional `step`, **tracks**, and **markers**
  (named times, used for events and script cues).
- **Track**: `(NodeId, Property) → [Key]`. Each key has a time, a value, and
  the easing *into* the next key: `Hold`, `Linear`, `CubicBezier(x1, y1,
  x2, y2)`, or named presets.
- **Instance time mode** (replaces the MovieClip/Graphic split with a
  per-instance setting):
  - `Synced { animation, offset, repeat }`: the child plays one of *its*
    animations on the parent's clock (Flash Graphic, and the "nested
    animations" trick).
  - `Free { animation }`: the child has its own clock and can be driven by
    its own mixer or state machine (Flash MovieClip).
- **Button** isn't its own type. It's a composition with `up`, `over`, and
  `down` animations and a hit node, plus a small built-in state machine (M6).

### Blending
- The runtime **mixer** plays one or more animations on a composition
  instance at once, each with a *weight*. Numeric properties are blended by
  weight:
  - position, scale, skew, and pivot: linear
  - rotation: shortest arc
  - colors: per channel, in linear space
- Discrete properties can't be blended: blend mode, visibility, the flipbook
  drawing, and the time mode's animation choice. They take the value from
  the highest-weight animation. A property that no active animation keys
  keeps its rest value.
- **Crossfade** is the first feature: switching animations over a duration.
  It covers walk→run and button hover. Later, and in the order Rive does
  them:
  - blend spaces (1D / 2D, e.g. walk↔run by speed)
  - additive layers
  - **state machines**: inputs, states, and transitions with durations,
    stored in the document
- The mixer state is part of the *runtime state*, not the document, so it
  serializes and survives a stage restart.

### Animatable properties
- **Transform:** position, rotation, scale, skew, and pivot (Flash's
  registration point).
- **Color:** opacity, and a color transform (multiply plus add, like Flash).
- **Visibility.**
- **Flipbook drawing index:** hold keys only.
- **Blend mode:** hold keys only.
- **Synced-instance offset.**
- **Later:** shape morphs, filter parameters, and text properties.

### Identity
- Every composition, node, animation, track, and asset has a stable `Id`.
  It's a random 64-bit value, written as short base32 in files, with one
  typed newtype per kind.
- Everything references by ID, never by index, so commands, undo, merges,
  and scripts' object handles all stay valid.
- Runtime objects are identified by their **instance path**: the chain of
  `Instance` node IDs from the root. That identity is stable for as long as
  the nodes exist, so script references and running playheads survive edits
  and restarts.

### Files
- **Project directory** `name.bs2d/`, so git merges and diffs stay small:
  - `project.ron`: settings, editor preferences (time grid, spatial grid),
    the library index, and the root composition ID
  - `compositions/<id>.ron`: one file per composition, holding its node tree
    and animations
  - `assets/`: bitmaps, fonts, and audio, referenced by path and content hash
- **Published movie**: a single binary file (postcard) with a version
  header, for the player.
- Format changes are caught by insta snapshots, the same way the protocol's
  wire format already is.

### File encoding
Conventions for `.ron` files, chosen for exactness and readable diffs:
- **Times** are exact strings:
  - `"2s"` and `"0.25s"` for decimal seconds
  - `"7/24s"` and `"1/60s"` for common frame-rate fractions
  - raw flicks otherwise, e.g. `"123f"`
- **Colors** are `"#rrggbb"` or `"#rrggbbaa"`: 8-bit sRGB with straight
  alpha.
- **Angles** are degrees. **Coordinates** are stage pixels, with y pointing
  down and the origin at the top left.
- **IDs** are a kind prefix plus 13 characters of Crockford base32, e.g.
  `node_0000002x12401`.
- **Defaults are omitted:** identity transforms, opacity 1, linear easing,
  and similar.
- The `implicit_some` and `unwrap_variant_newtypes` RON extensions are
  enabled in each file's header.
- **Output is canonical:** the same project always produces the same bytes.
- **Example:** `samples/bounce.bs2d/` in the repo is the reference sample
  and doubles as the format snapshot.

### Sketch
```rust
pub struct Project {
    pub settings: ProjectSettings,             // stage size, background, pixel-art mode
    pub editor: EditorPrefs,                   // time grid (ticks/s), spatial grid (px), snapping on/off
    pub root: CompId,
    pub compositions: BTreeMap<CompId, Composition>,
    pub assets: BTreeMap<AssetId, Asset>,
}
pub struct Composition {
    pub id: CompId,
    pub name: String,
    pub root: NodeId,
    pub nodes: BTreeMap<NodeId, Node>,         // tree via `children`
    pub animations: BTreeMap<AnimId, Animation>,
    pub default_animation: Option<AnimId>,
}
pub struct Node      { pub id: NodeId, pub name: String, pub kind: NodeKind, pub rest: Props, pub children: Vec<NodeId> }
pub enum   NodeKind  { Group, Shape(Shape), Flipbook(Vec<Drawing>), Bitmap(AssetId), Instance(Instance), Mask }
pub struct Instance  { pub comp: CompId, pub time: TimeMode }
pub enum   TimeMode  { Synced { animation: AnimId, offset: Time, repeat: Repeat }, Free { animation: Option<AnimId> } }
pub struct Animation { pub id: AnimId, pub name: String, pub duration: Time, pub looping: LoopMode,
                       pub step: Option<Time>, pub tracks: Vec<Track>, pub markers: Vec<Marker> }
pub struct Track     { pub node: NodeId, pub property: Property, pub keys: Vec<Key> }
pub struct Key       { pub at: Time, pub value: Value, pub ease: Ease }
pub struct Time(pub i64); // flicks
```

### Evaluation
`evaluate(&Project, &RuntimeState, now) -> Scene`:
1. Walk the node tree from the root composition, starting from rest values.
2. For each composition instance, sample its active animations (from the
   mixer state, or the parent's clock for synced instances) and blend them.
3. Apply script overrides.
4. Compose world transforms and color transforms.
5. Emit draw items in z-order.

## Consequences
- M1 builds this in order: types, RON I/O, `evaluate` with a single
  animation, then crossfade blending, then rendering the evaluated `Scene`
  with lyon. State machines and blend spaces come later (M6 alongside
  scripting, or earlier if interactivity needs them).
- `evaluate` is the most important function in the engine, and it is pure.
  It gets property tests: evaluating at the same time twice gives the same
  result, loop and animation boundaries behave correctly, easing endpoints
  are exact, and blending one animation at weight 1 reproduces it exactly.
- Commands (M2) edit `Project` by ID. Undo inverts them.
- The editor's timeline shows **one animation at a time**, with one row per
  node (like Flash's layer rows) and keys on that node's property tracks. An
  animation picker switches between them, like Rive and Spine.
- The tools process uses the same types for its panels.
- `docs/timeline-and-display-list.md` and the data-model sketch in
  `docs/architecture.md` get rewritten to match once this is accepted.

## Decisions (2026-09-28)
1. The reusable unit is a **Composition**.
2. **Named animations per composition, with blending** (Rive-style). The
   node tree is fixed, and animations key it. Crossfade comes first.
3. **No project frame rate.** The runtime follows the display's refresh
   rate, with an optional cap.
4. **Project directory** format.
5. **Top-level groups act as layers in the UI:** lock, hide, and outline.
   They're still ordinary nodes underneath.
6. **Flipbook drawings are stored inline** in the composition file, with IDs.
7. **Two separate snapping settings:**
   - **Time grid:** keys snap to *N* ticks per second. Default 60, and 30 is
     typical for classic games.
   - **Spatial grid:** objects snap to a pixel grid on the stage. Grid size
     is configurable.

   Neither setting affects playback speed. The retro *look* comes from
   per-animation `step`; the retro *pace* comes from the player's frame cap
   or, later, a fixed logic tick.
8. **Configurable stage size, and a pixel-art mode** for pixel-art projects.
