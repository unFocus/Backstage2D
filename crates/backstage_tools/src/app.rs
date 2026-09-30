//! The editor window: panels around a live stage section.

use crate::health::{AfterExit, StageHealth};
use crate::panels;
use crate::smoke::{SmokeStep, SmokeTest};
use crate::stage_view::StageView;
use crate::supervisor::{StageEvent, Supervisor};
use crate::working_copy::{CommitError, Recovery, WorkingCopy};
use backstage_core::{Command, Entry, Project};
use backstage_protocol::ToStage;
use gtk::{gdk, glib, prelude::*};
use relm4::prelude::*;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Project directory to edit; unset means the built-in sample. A stopgap
/// until File → Open (M3).
pub const PROJECT_ENV: &str = "BACKSTAGE_PROJECT";

pub struct App {
    supervisor: Supervisor,
    /// The editor's copy of the document; `None` if the project didn't open.
    copy: Option<WorkingCopy>,
    /// The current stage has replayed the log and matches the copy, so it
    /// can take edits.
    stage_ready: bool,
    stage_view: StageView,
    health: StageHealth,
    smoke: Option<SmokeTest>,
    status: String,
    banner: Option<String>,
    frames_this_second: u32,
    fps: u32,
}

#[derive(Debug)]
pub enum AppMsg {
    Stage {
        session: u64,
        event: StageEvent,
    },
    StageResized {
        width: u32,
        height: u32,
        scale: f64,
    },
    Pointer(Option<(f32, f32)>),
    /// User asked for a restart: forgives earlier crashes.
    RestartStage,
    /// Automatic restart after a crash.
    AutoRestartStage,
    KillStage,
    Tick,
    Undo,
    Redo,
    /// Debug edit until M4: move the first top-level node by this much.
    Nudge(f32, f32),
}

#[relm4::component(pub)]
impl SimpleComponent for App {
    /// Path to the `backstage_stage` binary.
    type Init = PathBuf;
    type Input = AppMsg;
    type Output = ();

    view! {
        gtk::ApplicationWindow {
            #[watch]
            set_title: Some(&model.title()),
            set_default_size: (1400, 900),

            #[wrap(Some)]
            set_titlebar = &gtk::HeaderBar {
                pack_start = &gtk::MenuButton {
                    set_icon_name: "open-menu-symbolic",
                    set_tooltip_text: Some("Menu (not implemented)"),
                },
                pack_start = &gtk::Button {
                    set_icon_name: "edit-undo-symbolic",
                    set_tooltip_text: Some("Undo (Ctrl+Z)"),
                    #[watch]
                    set_sensitive: model.can_edit(|c| c.document().can_undo()),
                    connect_clicked => AppMsg::Undo,
                },
                pack_start = &gtk::Button {
                    set_icon_name: "edit-redo-symbolic",
                    set_tooltip_text: Some("Redo (Ctrl+Shift+Z)"),
                    #[watch]
                    set_sensitive: model.can_edit(|c| c.document().can_redo()),
                    connect_clicked => AppMsg::Redo,
                },
                pack_end = &gtk::Button {
                    set_label: "Kill Stage",
                    set_tooltip_text: Some("SIGKILL the stage process to test crash recovery"),
                    connect_clicked => AppMsg::KillStage,
                },
                pack_end = &gtk::Button {
                    set_label: "Restart Stage",
                    connect_clicked => AppMsg::RestartStage,
                },
            },

            gtk::Paned {
                set_orientation: gtk::Orientation::Vertical,
                set_position: 660,
                set_shrink_end_child: false,

                #[wrap(Some)]
                set_start_child = &gtk::Paned {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_position: 240,
                    set_shrink_start_child: false,
                    set_start_child: Some(&panels::library()),

                    #[wrap(Some)]
                    set_end_child = &gtk::Paned {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_position: 880,
                        set_shrink_end_child: false,
                        set_resize_end_child: false,

                        #[wrap(Some)]
                        set_start_child = &gtk::Overlay {
                            #[local_ref]
                            stage_view -> StageView {
                                add_controller = gtk::EventControllerMotion {
                                    connect_motion[sender] => move |_, x, y| {
                                        sender.input(AppMsg::Pointer(Some((x as f32, y as f32))));
                                    },
                                    connect_leave[sender] => move |_| {
                                        sender.input(AppMsg::Pointer(None));
                                    },
                                },
                            },
                            add_overlay = &gtk::Label {
                                add_css_class: "backstage-stage-status",
                                set_halign: gtk::Align::Start,
                                set_valign: gtk::Align::End,
                                set_margin_all: 8,
                                #[watch]
                                set_label: &format!("{} · {} fps", model.status, model.fps),
                            },
                            add_overlay = &gtk::Label {
                                add_css_class: "backstage-stage-banner",
                                set_halign: gtk::Align::Center,
                                set_valign: gtk::Align::Center,
                                #[watch]
                                set_visible: model.banner.is_some(),
                                #[watch]
                                set_label: model.banner.as_deref().unwrap_or_default(),
                            },
                        },
                        set_end_child: Some(&panels::properties()),
                    },
                },
                set_end_child: Some(&panels::timeline()),
            },
        }
    }

    fn init(stage_binary: PathBuf, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        relm4::set_global_css(CSS);
        let (copy, load_error) = match open_project() {
            Ok(copy) => (Some(copy), None),
            Err(e) => (None, Some(e)),
        };
        let mut model = App {
            supervisor: Supervisor::new(stage_binary),
            copy,
            stage_ready: false,
            stage_view: StageView::default(),
            health: StageHealth::new(Instant::now()),
            smoke: SmokeTest::from_env(),
            status: "Stage starting…".into(),
            banner: None,
            frames_this_second: 0,
            fps: 0,
        };
        let stage_view = &model.stage_view;
        let widgets = view_output!();
        root.add_controller(shortcuts(&sender));

        let s = sender.clone();
        model.stage_view.connect_stage_resized(move |width, height, scale| {
            s.input(AppMsg::StageResized { width, height, scale });
        });
        let s = sender.clone();
        glib::timeout_add_seconds_local(1, move || {
            s.input(AppMsg::Tick);
            glib::ControlFlow::Continue
        });
        if model.smoke.is_some() {
            glib::timeout_add_local_once(crate::smoke::SMOKE_TIMEOUT, || {
                eprintln!("smoke: timed out");
                std::process::exit(1);
            });
        }
        match load_error {
            None => sender.input(AppMsg::RestartStage),
            Some(e) => {
                model.status = "No project".into();
                model.banner = Some(format!("Could not open the project:\n{e}"));
            }
        }

        ComponentParts { model, widgets }
    }

    fn shutdown(&mut self, _widgets: &mut Self::Widgets, _output: relm4::Sender<()>) {
        if let Some(copy) = self.copy.take() {
            copy.finish();
        }
    }

    fn update(&mut self, msg: AppMsg, sender: ComponentSender<Self>) {
        match msg {
            AppMsg::Stage { session, event } if session == self.supervisor.session() => {
                self.health.saw_event(Instant::now());
                self.handle_stage_event(event, &sender);
            }
            AppMsg::Stage { .. } => {} // from a stage we already replaced
            AppMsg::StageResized { width, height, scale } => {
                self.supervisor.send(&ToStage::Resize { width, height, scale });
            }
            AppMsg::Pointer(p) => self.supervisor.send(&ToStage::Pointer(p)),
            AppMsg::RestartStage => self.start_stage(&sender, true),
            AppMsg::AutoRestartStage => self.start_stage(&sender, false),
            AppMsg::KillStage => self.supervisor.kill(),
            AppMsg::Undo => self.submit(Entry::Undo),
            AppMsg::Redo => self.submit(Entry::Redo),
            AppMsg::Nudge(dx, dy) => self.update_nudge(dx, dy),
            AppMsg::Tick => {
                self.fps = std::mem::take(&mut self.frames_this_second);
                if self.health.is_stalled(Instant::now()) {
                    // Hung, not crashed: kill it and let the exit path restart it.
                    self.status = "Stage stopped responding".into();
                    self.supervisor.kill();
                }
            }
        }
    }
}

impl App {
    fn title(&self) -> String {
        // Edits since opening live only in the recovery log until Save (M3).
        let edited = self.copy.as_ref().is_some_and(|c| c.document().seq() > 0);
        if edited { "Backstage2D •".into() } else { "Backstage2D".into() }
    }

    /// Whether an edit can be sent now, and `f` allows it.
    fn can_edit(&self, f: impl Fn(&WorkingCopy) -> bool) -> bool {
        self.stage_ready && self.copy.as_ref().is_some_and(f)
    }

    /// Asks the stage to apply `entry`. The copy changes when it's committed.
    fn submit(&mut self, entry: Entry) {
        if !self.stage_ready {
            return;
        }
        if let Some(copy) = self.copy.as_mut() {
            self.supervisor.send(&copy.submit(entry));
        }
    }

    fn run_smoke_step(&mut self, step: Option<SmokeStep>) {
        match step {
            None => {}
            Some(SmokeStep::Nudge) => self.update_nudge(10.0, 0.0),
            Some(SmokeStep::Undo) => self.submit(Entry::Undo),
            Some(SmokeStep::Kill) => self.supervisor.kill(),
            Some(SmokeStep::Pass) => {
                self.supervisor.stop();
                std::process::exit(0);
            }
            Some(SmokeStep::Fail(why)) => {
                eprintln!("smoke: failed: {why}");
                std::process::exit(1);
            }
        }
    }

    fn update_nudge(&mut self, dx: f32, dy: f32) {
        if let Some(entry) = self.copy.as_ref().and_then(|c| nudge_entry(c.document().project(), dx, dy)) {
            self.submit(entry);
        }
    }

    fn start_stage(&mut self, sender: &ComponentSender<Self>, manual: bool) {
        self.stage_ready = false;
        self.health.started(Instant::now(), manual);
        self.status = "Stage starting…".into();
        let input = sender.input_sender().clone();
        let result =
            self.supervisor.start(move |session, event| input.emit(AppMsg::Stage { session, event }));
        if let Err(e) = result {
            self.status = "Stage failed to start".into();
            self.banner = Some(format!("Could not start stage: {e}"));
        }
    }

    fn handle_stage_event(&mut self, event: StageEvent, sender: &ComponentSender<Self>) {
        match event {
            StageEvent::Connected { pid, adapter } => {
                self.health.connected(Instant::now());
                self.banner = None;
                self.status = format!("Stage pid {pid} · {adapter}");
                // The new stage knows nothing yet: send it the document and
                // the current view state.
                if let Some(copy) = &self.copy {
                    self.supervisor.send(&copy.load_message());
                }
                if let Some((width, height, scale)) = self.stage_view.stage_size() {
                    self.supervisor.send(&ToStage::Resize { width, height, scale });
                }
            }
            StageEvent::FrameAvailable => {
                if let Some(frame) = self.supervisor.take_frame() {
                    let texture = gdk::MemoryTexture::new(
                        frame.width as i32,
                        frame.height as i32,
                        gdk::MemoryFormat::R8g8b8a8,
                        &frame.pixels,
                        frame.stride as usize,
                    );
                    self.stage_view.set_frame(texture.upcast());
                    self.frames_this_second += 1;
                    let step = self.smoke.as_mut().and_then(|s| s.frame(self.supervisor.session()));
                    self.run_smoke_step(step);
                }
            }
            StageEvent::Alive => {}
            StageEvent::Log(line) => eprintln!("stage: {line}"),
            StageEvent::Loaded { seq, hash } => {
                let Some(copy) = &self.copy else { return };
                let check = copy.check_loaded(seq, hash);
                match &check {
                    Ok(()) => self.stage_ready = true,
                    Err(e) => {
                        // Replaying again would give the same result, so
                        // warn instead of restarting, and take no edits. The
                        // editor's copy stays the reference.
                        eprintln!("stage document differs from the editor's: {e}");
                        self.banner = Some(format!("The stage's document differs from the editor's.\n{e}"));
                    }
                }
                let step =
                    self.smoke.as_mut().and_then(|s| s.loaded(self.supervisor.session(), seq, check.is_ok()));
                self.run_smoke_step(step);
            }
            StageEvent::Committed { seq, entry, .. } => {
                let Some(copy) = self.copy.as_mut() else { return };
                match copy.committed(seq, &entry) {
                    Ok(()) => {
                        let step = self.smoke.as_mut().and_then(|s| s.committed(seq));
                        self.run_smoke_step(step);
                    }
                    Err(CommitError::AutosaveFailed(e)) => {
                        eprintln!("autosave failed: {e}");
                        self.banner =
                            Some(format!("Autosave failed: {e}\nEdits since then are only in memory."));
                    }
                    Err(e @ CommitError::OutOfSync(_)) => {
                        // Restart the stage; it replays the editor's log.
                        eprintln!("{e}");
                        self.supervisor.kill();
                    }
                }
            }
            StageEvent::Rejected { request, reason } => {
                eprintln!("stage rejected request {request}: {reason}");
                self.status = format!("Edit rejected: {reason}");
            }
            StageEvent::Exited(reason) => {
                self.stage_ready = false;
                self.supervisor.reap();
                self.stage_view.clear_frame();
                if self.health.exited(Instant::now()) == AfterExit::GiveUp {
                    self.status = "Stage stopped".into();
                    self.banner =
                        Some(format!("Stage keeps crashing ({reason}).\nUse Restart Stage to retry."));
                    return;
                }
                self.status = format!("Stage down: {reason}");
                self.banner = Some(format!("Restarting stage…\n({reason})"));
                let s = sender.clone();
                glib::timeout_add_local_once(Duration::from_millis(750), move || {
                    s.input(AppMsg::AutoRestartStage);
                });
            }
        }
    }
}

/// Debug edit until M4: moves the first child of the root composition's
/// root node (the ground in the sample) by `(dx, dy)` stage pixels.
pub fn nudge_entry(project: &Project, dx: f32, dy: f32) -> Option<Entry> {
    let comp = project.root_composition()?;
    let node = *comp.root_node()?.children.first()?;
    let mut rest = comp.nodes.get(&node)?.rest;
    rest.transform.position.x += dx;
    rest.transform.position.y += dy;
    Some(Entry::Do(Command::SetRest { comp: comp.id, node, rest }))
}

/// Ctrl+Z / Ctrl+Shift+Z, and the arrow keys for the debug nudge (10 px, or
/// 1 px with Shift). Capture phase, so focused widgets don't swallow the
/// arrows.
fn shortcuts(sender: &ComponentSender<App>) -> gtk::ShortcutController {
    #[derive(Clone, Copy)]
    enum Action {
        Undo,
        Redo,
        Nudge(f32, f32),
    }
    let mut bindings =
        vec![("<Control>z".to_owned(), Action::Undo), ("<Control><Shift>z".to_owned(), Action::Redo)];
    for (key, dx, dy) in [("Left", -1.0, 0.0), ("Right", 1.0, 0.0), ("Up", 0.0, -1.0), ("Down", 0.0, 1.0)] {
        bindings.push((key.to_owned(), Action::Nudge(dx * 10.0, dy * 10.0)));
        bindings.push((format!("<Shift>{key}"), Action::Nudge(dx, dy)));
    }

    let controller = gtk::ShortcutController::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    for (trigger, action) in bindings {
        let s = sender.clone();
        let callback = gtk::CallbackAction::new(move |_, _| {
            s.input(match action {
                Action::Undo => AppMsg::Undo,
                Action::Redo => AppMsg::Redo,
                Action::Nudge(dx, dy) => AppMsg::Nudge(dx, dy),
            });
            glib::Propagation::Stop
        });
        controller
            .add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string(&trigger), Some(callback)));
    }
    controller
}

/// Opens the project to edit, from `BACKSTAGE_PROJECT` or the built-in
/// sample, with autosave to a fresh recovery directory if one can be made.
fn open_project() -> anyhow::Result<WorkingCopy> {
    use anyhow::Context;
    let project = match std::env::var_os(PROJECT_ENV) {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            backstage_core::load(&dir).with_context(|| format!("{}", dir.display()))?
        }
        None => backstage_core::sample::bounce(),
    };
    let recovery = Recovery::default_dir().and_then(|dir| Recovery::create(dir, &project));
    let recovery = match recovery {
        Ok(recovery) => Some(recovery),
        Err(e) => {
            eprintln!("autosave is off: cannot create a recovery directory: {e}");
            None
        }
    };
    let copy = WorkingCopy::new(project, recovery)?;
    if let Some(dir) = copy.recovery_dir() {
        eprintln!("autosaving edits to {}", dir.display());
    }
    Ok(copy)
}

const CSS: &str = "
.backstage-panel-title {
    font-weight: bold;
    padding: 6px 8px;
    opacity: 0.8;
}
.backstage-stage-status {
    background: rgba(0, 0, 0, 0.55);
    color: white;
    font-family: monospace;
    font-size: 0.85em;
    padding: 3px 8px;
    border-radius: 4px;
}
.backstage-stage-banner {
    background: rgba(20, 20, 24, 0.85);
    color: white;
    font-size: 1.2em;
    padding: 16px 24px;
    border-radius: 8px;
}
";

#[cfg(test)]
mod tests {
    use super::nudge_entry;
    use backstage_core::sample::{self, ids::*};
    use backstage_core::{Command, Entry, Props};

    #[test]
    fn nudge_moves_the_first_top_level_node_from_where_it_is() {
        let mut project = sample::bounce();
        let rest = |p: &backstage_core::Project| p.compositions[&STAGE].nodes[&GROUND].rest;
        let start = rest(&project).transform.position;
        let Some(Entry::Do(cmd @ Command::SetRest { comp: STAGE, node: GROUND, .. })) =
            nudge_entry(&project, 10.0, -1.0)
        else {
            panic!("expected a SetRest on the ground")
        };
        cmd.apply(&mut project).unwrap();
        let moved = rest(&project);
        assert_eq!((moved.transform.position.x, moved.transform.position.y), (start.x + 10.0, start.y - 1.0));
        assert_eq!(
            Props { transform: rest(&sample::bounce()).transform, ..moved },
            rest(&sample::bounce()),
            "only the position changes"
        );
    }
}
