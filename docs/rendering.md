# Rendering (wgpu)

## Phase 1: tessellated vectors
- Tessellate fills and strokes on the CPU with `lyon`. Cache the mesh per
  shape and reuse it across frames and instances.
- One uniform/instance buffer for transforms and color transforms. Batch draws
  by pipeline and texture.
- MSAA for anti-aliasing.
- Masks through the stencil buffer. Filters and blend modes through offscreen
  render targets.

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
