//! Backstage2D tools: the editor GUI (GTK 4 + Relm4).
//!
//! Hosts the panels and shows the stage section, whose frames come from a
//! separate `backstage_stage` process. See `docs/adr/0001-gui-framework.md`
//! and `docs/adr/0002-stage-process-isolation.md`.

pub mod app;
pub mod health;
pub mod panels;
pub mod smoke;
pub mod stage_view;
pub mod supervisor;
