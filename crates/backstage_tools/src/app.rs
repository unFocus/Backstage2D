//! The editor window: panels around a live stage section.

use crate::panels;
use crate::stage_view::StageView;
use crate::supervisor::{StageEvent, Supervisor};
use backstage_protocol::ToStage;
use gtk::{gdk, glib, prelude::*};
use relm4::prelude::*;
use std::time::{Duration, Instant};

/// No heartbeat or frame for this long means the stage is hung.
const STALL_TIMEOUT: Duration = Duration::from_secs(3);
/// Give up auto-restarting after this many crashes in quick succession.
const MAX_QUICK_CRASHES: u32 = 3;
const QUICK_CRASH_WINDOW: Duration = Duration::from_secs(5);

pub struct App {
    supervisor: Supervisor,
    stage_view: StageView,
    connected: bool,
    last_seen: Instant,
    started_at: Instant,
    quick_crashes: u32,
    status: String,
    banner: Option<String>,
    frames_this_second: u32,
    fps: u32,
}

#[derive(Debug)]
pub enum AppMsg {
    Stage { session: u64, event: StageEvent },
    StageResized { width: u32, height: u32, scale: f64 },
    Pointer(Option<(f32, f32)>),
    RestartStage,
    KillStage,
    Tick,
}

#[relm4::component(pub)]
impl SimpleComponent for App {
    type Init = ();
    type Input = AppMsg;
    type Output = ();

    view! {
        gtk::ApplicationWindow {
            set_title: Some("Backstage2D"),
            set_default_size: (1400, 900),

            #[wrap(Some)]
            set_titlebar = &gtk::HeaderBar {
                pack_start = &gtk::MenuButton {
                    set_icon_name: "open-menu-symbolic",
                    set_tooltip_text: Some("Menu (not implemented)"),
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

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        relm4::set_global_css(CSS);
        let model = App {
            supervisor: Supervisor::new(),
            stage_view: StageView::default(),
            connected: false,
            last_seen: Instant::now(),
            started_at: Instant::now(),
            quick_crashes: 0,
            status: "Stage starting…".into(),
            banner: None,
            frames_this_second: 0,
            fps: 0,
        };
        let stage_view = &model.stage_view;
        let widgets = view_output!();

        let s = sender.clone();
        model.stage_view.connect_stage_resized(move |width, height, scale| {
            s.input(AppMsg::StageResized { width, height, scale });
        });
        let s = sender.clone();
        glib::timeout_add_seconds_local(1, move || {
            s.input(AppMsg::Tick);
            glib::ControlFlow::Continue
        });
        sender.input(AppMsg::RestartStage);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: AppMsg, sender: ComponentSender<Self>) {
        match msg {
            AppMsg::Stage { session, event } if session == self.supervisor.session() => {
                self.last_seen = Instant::now();
                self.handle_stage_event(event, &sender);
            }
            AppMsg::Stage { .. } => {} // from a stage we already replaced
            AppMsg::StageResized { width, height, scale } => {
                self.supervisor.send(&ToStage::Resize { width, height, scale });
            }
            AppMsg::Pointer(p) => self.supervisor.send(&ToStage::Pointer(p)),
            AppMsg::RestartStage => {
                self.quick_crashes = 0;
                self.start_stage(&sender);
            }
            AppMsg::KillStage => self.supervisor.kill(),
            AppMsg::Tick => {
                self.fps = std::mem::take(&mut self.frames_this_second);
                if self.connected && self.last_seen.elapsed() > STALL_TIMEOUT {
                    // Hung, not crashed: kill it and let the exit path restart it.
                    self.status = "Stage stopped responding".into();
                    self.supervisor.kill();
                }
            }
        }
    }
}

impl App {
    fn start_stage(&mut self, sender: &ComponentSender<Self>) {
        self.connected = false;
        self.started_at = Instant::now();
        self.status = "Stage starting…".into();
        let input = sender.input_sender().clone();
        let result = self
            .supervisor
            .start(move |session, event| input.emit(AppMsg::Stage { session, event }));
        if let Err(e) = result {
            self.status = "Stage failed to start".into();
            self.banner = Some(format!("Could not start stage: {e}"));
        }
    }

    fn handle_stage_event(&mut self, event: StageEvent, sender: &ComponentSender<Self>) {
        match event {
            StageEvent::Connected { pid, adapter } => {
                self.connected = true;
                self.banner = None;
                self.status = format!("Stage pid {pid} · {adapter}");
                // The new stage knows nothing yet: send it the current state.
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
                }
            }
            StageEvent::Alive => {}
            StageEvent::Log(line) => eprintln!("stage: {line}"),
            StageEvent::Exited(reason) => {
                self.connected = false;
                self.supervisor.reap();
                self.stage_view.clear_frame();
                if self.started_at.elapsed() < QUICK_CRASH_WINDOW {
                    self.quick_crashes += 1;
                } else {
                    self.quick_crashes = 0;
                }
                if self.quick_crashes >= MAX_QUICK_CRASHES {
                    self.status = "Stage stopped".into();
                    self.banner =
                        Some(format!("Stage keeps crashing ({reason}).\nUse Restart Stage to retry."));
                    return;
                }
                self.status = format!("Stage down: {reason}");
                self.banner = Some(format!("Restarting stage…\n({reason})"));
                let s = sender.clone();
                glib::timeout_add_local_once(Duration::from_millis(750), move || {
                    s.input(AppMsg::RestartStage);
                });
            }
        }
    }
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
