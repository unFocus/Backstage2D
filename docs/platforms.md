# Platform support

Backstage2D targets modern systems only. There are no legacy fallbacks in
the editor.

| Platform | Editor | Player | Notes |
|---|---|---|---|
| Linux, **Wayland** (KDE Plasma 6, GNOME 46+) | **Primary, first** | Yes | No X11 support. XWayland isn't a goal. |
| macOS (current and previous major version) | Later | Yes | Metal through wgpu |
| Windows 11 | Later | Yes | DX12 through wgpu. Vulkan as an option. |
| Web (WebGPU) | No | Maybe | Player only, if the renderer port is worth it |
| X11, Windows 10, older macOS | No | Not planned | |

## What this lets us assume
- Vulkan 1.3 / Metal 3 / DX12-class GPUs. We can require compute shaders.
- Wayland protocols: `xdg-shell`, `linux-dmabuf`, fractional scaling,
  `xdg-foreign` (both KWin and Mutter support it).
- DMA-BUF texture sharing on Linux, IOSurface on macOS, and DXGI shared
  handles on Windows are all available. This makes zero-copy stage
  frames possible (ADR 0002 option C).
- Consistent HiDPI and fractional scaling in every layer.
