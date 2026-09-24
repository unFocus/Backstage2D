//! Backstage2D tools: the editor GUI (GTK 4 + Relm4).
//!
//! Hosts the panels and shows the stage section, whose frames come from a
//! separate `backstage_stage` process. See `docs/adr/0001-gui-framework.md`
//! and `docs/adr/0002-stage-process-isolation.md`.

mod app;
mod panels;
mod stage_view;
mod supervisor;

fn main() {
    relm4::RelmApp::new("dev.backstage2d.Tools").run::<app::App>(());
}
