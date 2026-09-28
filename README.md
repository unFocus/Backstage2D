# Backstage2D

[![CI](https://github.com/unFocus/Backstage2D/actions/workflows/ci.yml/badge.svg)](https://github.com/unFocus/Backstage2D/actions/workflows/ci.yml)

A 2D animation and interactive-content engine in Rust. It borrows the Flash Pro
model (stage, timeline, symbols, display list) and updates it with modern
ideas. Rendering goes through wgpu.

Status: pre-alpha. The editor mockup runs, with a live stage process;
see the [roadmap](docs/roadmap.md). The original 2012 AS3 Backstage2D is
preserved on the [`archive`](https://github.com/unFocus/Backstage2D/tree/archive)
branch. Start with [`docs/`](docs/README.md).

## Layout

| Path | Purpose |
|---|---|
| `crates/backstage_core` | Document model, display list, timeline. No GPU or GUI dependencies. |
| `crates/backstage_render` | wgpu renderer for the display list. |
| `crates/backstage_script` | Scripting language and VM. |
| `crates/backstage_protocol` | Messages and shared-memory frames between stage and tools. |
| `crates/backstage_stage` | Editing engine process: renders the stage and streams frames. (runtime side) |
| `crates/backstage_player` | Standalone runtime that plays published content. |
| `crates/backstage_tools` | Editor GUI (GTK 4 + Relm4). Starts and supervises the stage. (editor side) |
| `docs/` | Vision, architecture, and decision records. |


## Building

See [docs/dev-setup.md](docs/dev-setup.md) for the GTK 4 dev files.

```sh
export PKG_CONFIG=/home/linuxbrew/.linuxbrew/bin/pkgconf   # Homebrew GTK on Bazzite
cargo build --workspace
cargo run -p backstage_tools     # editor; starts backstage_stage itself
```
