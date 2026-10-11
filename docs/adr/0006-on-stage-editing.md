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
