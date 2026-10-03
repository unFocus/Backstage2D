# ADR 0005: Edit state lives in the editor; the stage mirrors it

**Status:** Accepted (2026-10-02). Amends
[ADR 0002](0002-stage-process-isolation.md) Decision 1, which had the stage
owning the selection.

## Context
ADR 0002 put the selection in the stage, next to the document and the
in-progress gesture. M3 built the panels (timeline, Layers, Properties,
Library), and the selection ended up in the editor, where every panel
lives. Before M4 adds on-stage picking and transform handles, both sides
need to know the selection, and one of them has to own it.

The stage doesn't write files. It decides the order of commits to the
document. The editor saves from its own copy, which is hash-checked to
match the stage's (ADR 0004). Exports will also come from the document.
So neither saving nor exporting needs the stage to own editor state.

## Decision

### Three kinds of state
1. **The document.** Content, plus the editor settings saved with it
   (hide/lock/outline flags, the time grid).
   - It changes only through commands, and the stage decides their order
     (ADR 0004).
   - It's saved and exported.
2. **Edit state.** The selection, the playhead, the edited composition (the
   breadcrumb), and the expanded tree nodes.
   - It's per session: never saved and never undone.
   - **The editor owns it.**
   - The stage gets a mirror of what it needs: `ToStage::Transport` (the
     edited composition and the playhead) and `ToStage::Selection`. Both
     are sent on every change and again on every connect, so a restarted
     stage comes back with them.
3. **Stage state.** Hover, and the gesture in progress (a drag).
   - It's short-lived and owned by the stage.
   - It's lost if the stage crashes, as ADR 0002 §3 already allows.

### The selection
- `{ comp, nodes }`: an ordered list of nodes in the edited composition. The
  last entry is the **primary**.
  - Properties shows the primary.
  - Multi-select UI comes with M4 (marquee, shift-click). The protocol
    carries a list from the start.
- The stage's mirror ignores a selection for a composition other than the
  one it shows, and nodes that no longer exist (`Selection::shown`).
- Commands stay addressed by explicit IDs. An operation on "the selection"
  is turned into node IDs by the editor before it's submitted.

### Picking on the stage (M4)
Decided now, so M4 only has to build it:
1. The stage hit-tests the click or marquee, since it has the geometry.
2. It applies the result to its mirror **immediately**, so handles appear
   on its next frame with no added latency.
3. It reports `ToTools::Picked { nodes, mode: Replace | Add | Toggle }`.
4. The editor applies that to its selection and sends `Selection` back.
   The echo matches what the stage already shows.

If both sides change the selection at the same time, the editor's
`Selection` wins, because the editor is the authority. A drag acts on the
nodes the stage showed as selected when it started.

## Alternative considered: the stage owns the selection
**For:**
- One hub for any future clients in several processes.
- Commands on "the selection" could be worked out at commit time, in
  commit order.
- On-stage drags are always in step with the authority, with no echo.
- It matches ADR 0002 as first written.

**Against:**
- Every panel click waits for the stage. The stage reads messages once per
  tick, so panels would lag by up to a frame plus the reply.
- Nothing can be selected while the stage is down, restarting, or never
  started (a project that failed to load).
- The selection dies with the stage, unless the editor keeps a copy. That
  is this decision again, with the authority on the side that crashes.
- Session state ends up next to the document, which deliberately excludes
  it.

All panels live in the editor process, so a single owner there already
reaches every panel (`refresh_panels`). Stage ownership wouldn't add
listeners, only a hop before them.

**Revisit if** panels in several processes, or collaborative editing,
become a goal. The stage would then probably be the hub for both the
document and the selection. The messages are symmetric (`Selection` one
way, `Picked` the other), so moving the authority would stay a local
change.

## Consequences
- **Protocol v5** adds `ToStage::Selection`. `Picked` comes with M4.
- **Editor:** `App.selection` is a `Vec<NodeId>`. `SelectionSync` sends it
  when it changes, and again after a stage connects.
- **Stage:** `selection::Selection` holds the mirror. Nothing draws it yet;
  M4's handles will.
- The arrangement matches `Transport`. Both are mirrors of edit state, so
  a restarted stage needs nothing beyond `Load`, `Transport`, and
  `Selection`.
