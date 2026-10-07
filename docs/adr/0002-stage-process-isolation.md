# ADR 0002: Stage engine in its own process, tools as a separate view

**Status:** Accepted.
- **Implemented:** the process split, crash recovery, and display option B
  (M0.5). The stage rendering the real project (M1). Document authority,
  the editor's copy, and command replay after a restart (M2, see
  [ADR 0004](0004-commands-and-document-authority.md)).
- **M3:** the tools panels work entirely on the editor's document copy,
  as the Consequences below predicted, so no query hooks were needed. The
  playhead and the edited composition go to the stage with
  `ToStage::Transport`.
- **Amended by [ADR 0005](0005-edit-state.md):** edit state (selection,
  playhead, edited composition) is owned by the tools process, and the
  stage keeps a mirror (`Transport`, `Selection`). Decision 1's
  "selection" below is superseded.
- **Still to build:** the on-stage edit hooks (M4), and query hooks if
  they turn out to be needed.

## Terms
- **Stage (engine):** the `backstage_stage` process. It renders the display
  list and hosts **on-stage editing**: selection, bounding boxes, transform
  handles, bezier/path editing, onion skinning, snapping, and hit testing.
- **Tools:** the `backstage_tools` GUI process. It provides the timeline,
  library, properties, layers, and menus. It is *another view* of the state
  the stage holds.
- **Test Movie:** a new `backstage_player` process built from the document.
  It runs scripts.

## Context
We want the part most likely to fail (GPU work, rendering, and eventually
user code) isolated so it can crash or be restarted without losing the
user's work or bringing down the tools UI.

## Decision

### 1. The stage owns the live state and edit interaction
The stage process holds the working document, the derived display list,
selection, and the in-progress edit gesture. On-stage tools are part of the
engine. They draw overlays in the same pass as the content and use the real
geometry for hit testing, so the editor never keeps a second copy of the
geometry.

### 2. Every edit is a command, and the stage decides the order
Edits from on-stage gestures and from tools panels both become commands
against the **document**. The stage is the single writer. It applies each
command, gives it the next sequence number, and records it in the undo log.

Edit hooks *read* the display list for geometry and hit testing, and
*write* only to the document. For example, dragging an object in the middle
of a tween edits or creates a keyframe. It doesn't change the temporary
display object.

### 3. The tools process keeps a copy of every committed change
The stage broadcasts each committed command, with its sequence number, to
the tools process. The tools process keeps a matching copy of the document
and autosaves the command log to disk. The stage can crash without losing
data:

1. The tools process sees the crash (process exit or missed heartbeats).
2. It restarts the stage and sends the latest snapshot plus the command log.
3. Anything uncommitted is lost. That's at most the gesture in progress.

### 4. The editing stage doesn't run user scripts
In edit mode the stage can scrub and play timelines, but it never runs user
scripts. Test Movie runs scripts in a separate player process built from the
current document, as Flash did. A runaway script can only kill the player.

## Hooks (stage ↔ tools protocol)
- **Query:** document tree, display list at the playhead, bounds, selection,
  and hit tests at a point.
- **Subscribe:** committed commands, diagnostics, and heartbeats.
- **Mirror** (tools → stage, ADR 0005): the edited composition and
  playhead (`Transport`) and the selection (`Selection`), sent on change
  and on connect.
- **Report** (stage → tools, M4): on-stage picks (`Picked`), which the
  tools process makes the selection.
- **Mutate:** submit a command, set the active tool and its parameters.
- **Frames:** rendered stage output, when the stage is shown inside the
  tools window (see below).

Message types live in a `backstage_protocol` crate. They are versioned from
day one and serialized with serde and a binary format (postcard/bincode) over
a local socket.

## Showing the stage
Wayland has no way to embed another process's window (XEmbed is X11-only).
Because overlays are drawn in the engine, **every drag makes a round trip
between the processes**. How the stage appears therefore affects how
responsive editing feels.

| Option | How | Drag latency | Notes |
|---|---|---|---|
| **A. Separate stage window** | The stage opens its own window and receives input directly. `xdg-foreign` keeps it tied to the tools window. | **Lowest** (same as a single process) | Clients can't position windows on Wayland, so the compositor decides where they go. Feels like a multi-window app (Flash on Mac, GIMP). |
| **B. Shared-memory frames** | The tools process forwards input. The stage reads back each frame into shared memory, and the tools process uploads it. | About 2–3 frames at 60 Hz | Simplest way to show the stage inside the tools window. Works with any toolkit. |
| **C. Zero-copy (DMA-BUF / IOSurface / DXGI)** | The stage exports its render target. The tools process imports and composites it. | About 1–2 frames | Different code for each platform. **The toolkit must be able to import external textures**, which is a toolkit selection criterion. |

**Decision:** the stage is a section *inside* the tools window, so **B comes
first**. It is implemented in the M0.5 mockup:
- The stage renders offscreen and reads back into a 3-slot ring in
  `$XDG_RUNTIME_DIR/backstage2d/`.
- Each slot is protected by a seqlock, so the tools process drops a frame
  the stage has already overwritten instead of showing it torn.
- The tools process only ever keeps the newest frame and shows it as a
  `GdkMemoryTexture`.

**C** is the planned upgrade. On Linux it's concrete: export the render
target as DMA-BUF, then import it with `GdkDmabufTexture` and pass it to the
compositor with `GtkGraphicsOffload` (GTK ≥ 4.14). The frame protocol keeps
"which buffer is ready" separate from how pixels are transported, so C can
replace B. **A** remains only as a possible debug mode.

### Measured latency (option B, 2026-09-24)
`latency_probe` (see [testing.md](../testing.md)) on the RX 6800 (RADV)
measures from sending `Pointer` to a published frame showing the
crosshair: **min 7.8 ms, median 16.6 ms, p95 17 ms**.
- **Why it's about one frame:** the stage renders on a fixed 60 Hz tick, and
  the probe sends right after a frame, so it always waits almost a full
  tick. That makes the median the worst case, not the typical case.
- **Not included:** the GTK texture upload and the compositor, which add
  roughly 1–2 more display frames.
- **Improvements, when drag feel needs it:** render as soon as input
  arrives instead of waiting for the tick, then switch to zero-copy (C).

### Rendering on input (M4, 2026-10-07)
The stage now draws when something changes instead of on a fixed tick:
- **Input, edits, the transport, the selection, or a resize:** it handles
  everything queued, then draws one frame. A burst of pointer moves makes
  one frame.
- **While playing:** it also keeps the 60 Hz tick.
- **Paused with nothing changing:** it draws nothing (heartbeats only).

The probe, now run on a paused stage as the editor starts it: **min 0.9 ms,
median 0.9 ms, p95 1.0–1.6 ms** on the RX 6800 (lavapipe: median 3.5 ms).
That's the readback plus the socket round trip. The GTK upload and the
compositor are still not included, and they are now most of the delay,
which makes zero-copy (C) the next improvement.

## Consequences
- The tools process never links wgpu, the renderer, or the VM. It only
  speaks the protocol.
- The tools process doesn't need shape geometry. The timeline, library, and
  property panels work entirely on the document copy.
- The command-based document model and undo log are required from the start.
- Because on-stage editing lives in the engine, a different front end (web,
  another toolkit) could reuse it later.
