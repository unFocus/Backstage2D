# ADR 0006: On-stage editing: drags are stage gestures that commit commands

**Status:** Accepted (2026-10-11). Builds on
[ADR 0002](0002-stage-process-isolation.md) (edit hooks live in the engine),
[ADR 0004](0004-commands-and-document-authority.md) (the stage is the
document's only writer), and [ADR 0005](0005-edit-state.md) (gestures are
stage state).

## Context
M4 makes the stage an editing surface:
- clicking and marquee selection (steps 2–3)
- moving what's selected by dragging it (step 4)
- scale and rotate handles (step 5)

Three questions came up:
- who runs a drag
- what it shows while it's under way
- where its values go when the property is animated

## Decision

### The stage runs the gesture
The stage handles the press, the moves, and the release. It has the
geometry, and it draws the result on its next frame without a round trip.
A gesture is **stage state** (ADR 0005): short-lived, owned by the stage,
and lost if the stage crashes (at most the drag in progress).

- **Starting a move:** a press on a node that is selected once the press
  has picked. That's any plain press on a node, or one already in a
  multi-selection. It moves **every** selected node.
- **Going live:** after 3 logical pixels of movement (the same threshold as
  the marquee). A press and release in place is a click and edits nothing.
- **Releasing** commits **one** command (a `Batch` for several nodes or
  properties). One drag is one undo step.

### A preview on a copy of the document
When the drag goes live, the stage clones the document. On each move it:
- undoes the previous provisional command on the clone (with its inverse)
- applies the new one, built against the real document
- draws the clone, so boxes and hover follow it too

**It's exact:** it goes through the same `evaluate` as everything else,
keys included. The renderer's mesh cache is keyed per project, so it clears
once when drawing switches to the clone and once when it switches back.

### The stage commits its own command
The stage is the only writer (ADR 0004), so it applies the drag's command
itself and broadcasts `Committed { request: None }`. The editor's
`WorkingCopy` takes it like any other commit: it autosaves it, and Undo
reverses it.

### Where values go: keys on animated properties
`backstage_core::edit` holds the rule, so the stage's drags and the
editor's arrow-key nudges write the same thing:
- **A property with a track in the shown animation** gets a key at the
  playhead's animation-local time. A key already there keeps its ease; a
  new one is linear.
- **Any other property** changes the node's rest value.
- **Each property is decided on its own.** A node keyed on X but not Y gets
  a key for X and a rest change for Y.

This is ADR 0002's "dragging an object in the middle of a tween edits or
creates a keyframe". Flash's motion tweens behaved the same way.

The key time is taken **when the drag goes live**, not at the press. A pause
of playback caused by the press has arrived by then.

### The delta is mapped into each node's parent space
`evaluate` now records each node of the evaluated composition, groups
included, in `Scene.nodes` with its evaluated props and world transform.
`edit::move_nodes`:
- maps the pointer's stage-space delta through the inverse of each parent's
  world linear part
- adds it to the node's position at the playhead

A node inside a scaled or rotated group follows the pointer exactly. A
node whose ancestor is also selected is left to move with it.

### Cancelling
A move ends without committing on any of these:
- `CancelGesture`. The editor sends it on Escape **while its own drag
  gesture on the stage section is held**. Otherwise Escape deselects as
  before, so the editor needs no state from the stage.
- Any commit, since the preview was built on the old document.
- A `Load`.
- A `Selection` that differs from the stage's mirror. The editor's echo of
  the stage's own pick is equal, so it doesn't cancel.

A `Transport` mid-drag doesn't cancel. The preview follows it, and the key
time is already fixed.

### Handles (M4 step 5)
**One selected node** gets free-transform handles:
- **The box:** the node's own bounds, measured in its own space, so it turns
  and skews with the node. It's padded 2 logical pixels on screen.
- **8 scale handles** at the corners and edge middles.
- **Rotate zones** within 16 logical pixels outside the corners.
- **The pivot** (registration point), drawn as a diamond.

**Hit priority:** handle, then rotate zone, then the node (a move), then
empty space (a marquee).

**Scaling:**
- `edit::scale_node` keeps the **opposite handle** fixed. Alt keeps the
  pivot fixed instead.
- The dragged handle follows the pointer.
- The position compensates, so the anchor doesn't move.
- An edge handle changes one axis. Shift on a corner keeps the aspect ratio.

**Rotating:** `edit::rotate_node` turns about the pivot by the angle the
pointer sweeps around it, and Shift snaps the result to 15°.

**Several selected nodes:**
- Each keeps its own box. They share one stage-aligned box, padded 6
  pixels so it clears theirs, with **corner handles only**, rotate zones
  past its corners, and its centre as the pivot.
- **A corner scales them all evenly** about the opposite corner (Alt: the
  centre).
- **Past a corner turns them all** about the centre. Shift snaps the turn
  to 15° steps; the nodes may start at different angles.
- `edit::transform_nodes` moves each node's pivot point through the same
  turn and scale, adds the angle to its rotation, and multiplies its scale.
- **This is exact when the nodes' parents neither skew nor scale
  unevenly.** Under such a parent, a node's own rotation and scale can't
  express the shared transform, and it drifts. Non-uniform scale is
  single-node only for the same reason.

Handle drags write X, Y, ScaleX, ScaleY, and Rotation through `set_values`,
so they key animated properties like a move, with the same preview,
commit, and cancellation.

**Cursors:**
- The stage reports `ToTools::Cursor(CursorKind)` for what's under the
  pointer, only when it changes (protocol v9):
  - **default** over nothing
  - **pointer** over a node to pick
  - **move** over a selected one
  - a **resize arrow** along a handle's direction on screen, so it turns with
    the node
  - **crosshair** for rotate, since there's no standard rotate cursor
- The editor sets it on the stage section.
- The stage's own pointer crosshair, a debug aid since M0.5, is gone. The
  latency probe now watches the hover box instead.

### Snapping (M4 step 6)
- **One switch:** `EditorPrefs.spatial_snap` turns snapping on and shows
  the grid. `spatial_grid` is the spacing. The Properties panel's document
  view edits both, and Ctrl+' toggles snapping. They're saved with the
  project, like the time grid.
- **Where the grid sits:** through the edited composition's origin. That's
  the stage's corner for the root, and the stage centre for an entered
  composition.
- **Moves:** the **primary** node's pivot lands on a grid point
  (`edit::snapped_move`), and the rest of the selection moves by the same
  amount.
- **Scale handles** (one node or a shared box): the point the handle is
  dragged to snaps, so the shape's true corner lands on the grid.
  - Handles are measured on the true bounds. The 2 or 6 px padding is only
    how the box is drawn.
  - A press keeps its offset from the handle, so nothing jumps when you
    grab it.
- **Not snapped:** rotation (Shift has 15° steps), the marquee, and arrow
  nudges.

### Arrow keys
They nudge the selection by 1 stage pixel, or 10 with Shift (Flash's
convention). The editor does this with the same `edit::move_nodes`,
submitting the command as usual, so nudges key animated properties too.
They replaced the debug nudge.

## Alternatives considered
- **A preview that overrides the drawn scene** (offset each moved item's
  transform) instead of cloning the document. It's cheaper, but not exact
  through keys or for nested content, and it would need a second code path
  for every future handle. The clone costs one copy per drag.
- **The editor runs drags** (with the stage sending positions back). ADR
  0002 put on-stage tools in the engine, so that a drag is drawn with no
  round trip. Nothing here argues against that.
- **Separate commits per move** with coalescing. That gives one undo step
  only with a merge rule, and floods the log and the autosave. One commit
  on release is simpler.

## Consequences
- `Scene` has `nodes` (the evaluated composition's own nodes).
  `evaluate` stays pure.
- `backstage_core::edit` (`At`, `set_values`, `move_nodes`) is the shared
  home for direct-manipulation edits. Step 5's handles will add rotate and
  scale there.
- The editor's Properties panel still edits rest values. Switching it to
  `set_values`, so it also keys animated properties, is a small follow-up.
- Projects are cloned once per drag. If documents grow large enough for
  that to matter, the preview can switch to applying and undoing commands
  on the document itself.
