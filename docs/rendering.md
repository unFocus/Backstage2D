# Rendering (wgpu)

## Phase 1: tessellated vectors *(implemented, M1)*
- **Input:** `backstage_core::evaluate`'s `Scene`. `Renderer::render` takes
  a `Frame` holding the project, the scene, the scale, and the pointer.
- **Tessellation:** lyon turns each `Shape` into one mesh. Fills use the
  non-zero rule; strokes use their width, caps, joins, and miter limit. The
  tolerance is 0.05 local units.
- **Mesh cache:** each mesh is built once, keyed by the shape's address in
  the loaded project. The cache clears when the project changes.
- **Paints:** solid, linear, and radial gradients (up to 8 stops),
  evaluated per fragment from local coordinates, with clamped spread.
- **Per item:** world transform, opacity, and color transform
  (`c·multiply + add`), all instanced.
- **Color:** authored sRGB, blended in sRGB space (like Flash and the CSS
  default). Straight alpha.
- **Anti-aliasing:** 4× MSAA, resolved into the caller's texture view.
- **Framing** (`Presentation`):
  - **Editor:** the project's stage, fitted into the viewport with a margin,
    a drop shadow, and the background color. It's never enlarged past 1
    stage unit per logical pixel. The pointer crosshair is drawn on top as
    an engine-side editor overlay (ADR 0002).
  - **Player:** the stage fills the window, keeping its aspect ratio,
    between black bars. In pixel-art mode it scales by whole numbers once
    the window is big enough.
- **Pixel-art mode:** item translations are rounded to whole physical
  pixels.
- **Not yet:**
  - bitmaps (skipped)
  - blend modes other than Normal
  - masks (skipped by `evaluate`)
  - isolated group opacity: overlapping children of a half-transparent
    group each blend on their own, as in Flash without "cache as bitmap"

## Phase 2: options to evaluate
- **Vello** (compute-based vector rendering on wgpu): high quality, no
  tessellation cache. It needs compute shaders, which affects the WebGL
  fallback.
- Analytic AA or SDF for text.

## Render targets
The renderer draws into a caller-provided `wgpu::TextureView`. The player
passes the swapchain view. In editor mode, the stage process renders
offscreen and streams frames to the editor
([ADR 0002](adr/0002-stage-process-isolation.md)).
