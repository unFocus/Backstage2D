# ADR 0001: Editor GUI framework

**Status:** Accepted: GTK 4 + Relm4

## Context
The editor needs dockable panels, a dense custom timeline widget, tree and
list views, and property inspectors. It also needs a **stage viewport**.

[ADR 0002](0002-stage-process-isolation.md) moves the stage, including
on-stage editing overlays, into a separate engine process. The tools GUI
never draws stage geometry. At most it shows a stage image and forwards
input. **Sharing a wgpu device no longer counts as a benefit.** Target
platforms are listed in [platforms.md](../platforms.md): Wayland (KDE and
GNOME) first, then macOS and Windows 11. The criteria are now:

1. **Stability:** API churn, maturity, maintainer commitment.
2. **Flexibility:** custom widgets (the timeline is the hardest one), docking,
   theming.
3. **Streamed image performance:** can we update a texture every frame
   cheaply, and later import a GPU-shared texture (ADR 0002 option C)?
4. **Platforms:** native Wayland support that works well on both KWin and
   Mutter (fractional scaling, client-side decorations on GNOME), then macOS
   and Windows 11. X11 doesn't matter. Linking C/C++ is acceptable but costs
   something.

## Candidates

### egui (+ egui_dock)
- Stability: widely used, but releases often, with breaking but mostly
  mechanical changes. Pre-1.0.
- Flexibility: **excellent for custom widgets**. The timeline is just painter
  calls. `egui_dock` provides docking.
- Streamed image: `TextureHandle::set` each frame is easy. Zero-copy is
  possible through its wgpu backend.
- Downsides: tool-like look unless themed. Immediate mode redraws on
  interaction (cheap in practice).

### Slint
- Stability: **the best of the pure-Rust options**. 1.x since 2023, with a
  semver API promise and a funded company behind it.
- Flexibility: good standard widgets. Very custom widgets (the timeline) are
  harder: you build them from `Path`/`Rectangle` elements or draw them in Rust
  into an image. No docking out of the box.
- Streamed image: `Image::from_rgba8` per frame works. GPU texture import
  exists but is marked unstable.
- Other: `.slint` markup plus live preview. License is GPLv3,
  royalty-free (desktop), or commercial. Check that it fits.

### GPUI (Zed)
- ADR 0002 removed its main drawback (its own renderer).
- Stability: **the weakest**. Its API changes to suit Zed, the docs are thin,
  and there's no stability promise.
- Flexibility: very capable (Zed's own UIs are proof). No ready-made docking
  or timeline widgets, but the building blocks are good. Polished text.
- Streamed image: supports images, but it's unclear how fast per-frame
  updates are. Needs a spike.

### iced
- Stability: pre-1.0, with significant changes between releases.
- Flexibility: the `canvas` widget works for the timeline. `pane_grid` is
  only partial docking.
- Middle of the pack on every criterion.

### Qt 6 (cxx-qt) / GTK 4 (gtk-rs), native toolkits
- Stability: **the best overall**. Decades of maturity.
- Qt: excellent docking (`QDockWidget`) and custom painting, but brings in a
  large C++ build and LGPL obligations. GTK 4: great on Linux, weaker on
  macOS and Windows.
- Rust bindings add friction in both cases.

## Decision
**GTK 4 through gtk4-rs, with Relm4** to cut down GObject boilerplate.

- **Why GTK 4:**
  - Its API is the most stable of the candidates.
  - It's native on Wayland (GNOME and KDE).
  - It has built-in DMA-BUF import (`GdkDmabufTexture`, `GtkGraphicsOffload`),
    which is exactly what the stage section needs for zero-copy frames later.
- **Licensing:** GTK is LGPL-2.1+, which is fine with the normal dynamic
  linking. gtk4-rs and Relm4 are MIT (Relm4 is MIT/Apache-2.0). The user
  ruled out GPL-only options.
- **Why Relm4:** checked on 2026-09-24.
  - Relm4 0.11.0 (April 2026) targets gtk4-rs 0.11 and is actively
    maintained.
  - Its `view!` macro and components replace most hand-written widget code.
    Optional `libadwaita` and `libpanel` features give us a route to docking
    later.
  - vgtk (abandoned) and relm (GTK 3 only) were excluded.
- **Custom widgets:** the stage view and, later, the timeline are gtk4-rs
  subclasses (`#[glib::object_subclass]`). The `glib::Properties` and
  `CompositeTemplate` macros can reduce that code where it grows.
- **API level:** we build with gtk4-rs's `v4_14` feature. That limits us to
  GTK 4.14 APIs, so the binary runs on the system GTK (4.22 on the dev host)
  even when it's built against newer headers.

### Alternatives not chosen
- **GPUI:** Apache-2.0, polished, and strong on macOS/Windows. But its API
  changes to suit Zed, its docs are thin, and much of Zed's higher-level UI
  (docking, workspace) is GPL.
- **egui / iced:** good for custom widgets, but pre-1.0 churn, and sharing
  wgpu no longer matters (ADR 0002).
- **Slint:** the most stable Rust-native API. Excluded when the choice was
  narrowed to GPUI vs GTK. Its licenses are GPLv3, a royalty-free license
  with attribution terms, or commercial.
- **Qt:** mature, but a heavy C++ build and extra binding friction.
