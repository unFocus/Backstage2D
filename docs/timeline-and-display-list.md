# Timeline & display list

## Timeline semantics
- Frame-based at the document frame rate. Tween math runs in continuous time,
  so a 24fps document can still render smoothly at 120Hz.
- Layers: normal, mask, guide, folder. Also consider "adjustment" layers for
  filters.
- Tweens: property curves per keyframe span (position, scale, rotation, skew,
  color, filter params). Shape tweens are a separate, later feature.
- Labels and frame scripts attach to frames.
- Nested timelines advance independently (MovieClip) or stay synced to the
  parent (Graphic).

## Display list semantics
- A tree of `DisplayObject`s with depth ordering inside each container.
- Timeline-placed objects keep a stable identity across keyframes when the
  same instance continues. This matters for scripts holding references.
- Scripts can add, remove, and reparent objects. We need a rule for what
  happens when the timeline and a script disagree. Flash's rule
  ("script-touched objects detach from the timeline") is a reasonable start.

## Open questions
- Can a script-created object ever be taken back by the timeline?
- Event model: capture/bubble like AS3, or simpler?
- Hit testing: shape-accurate or bounds-only by default?
