//! The editor window: panels around a live stage section.

use crate::health::{AfterExit, StageHealth};
use crate::panels;
use crate::recovery::{self, Candidate, Recovery};
use crate::smoke::{self, SmokeStep, SmokeTest};
use crate::stage_view::StageView;
use crate::supervisor::{StageEvent, Supervisor};
use crate::timeline::{Playhead, Timeline, TimelineModel, TimelineMsg, TimelineOutput, pick_animation};
use crate::working_copy::{self, CommitError, SaveToError, WorkingCopy};
use backstage_core::{Animation, Command, EditorPrefs, Entry, Project, Time, TimeGrid};
use backstage_protocol::ToStage;
use gtk::{gdk, gio, glib, prelude::*};
use relm4::actions::{AccelsPlus, RelmAction, RelmActionGroup};
use relm4::prelude::*;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Project directory to open at startup; unset means the built-in sample.
pub const PROJECT_ENV: &str = "BACKSTAGE_PROJECT";

relm4::new_action_group!(WinActions, "win");
relm4::new_stateless_action!(OpenAction, WinActions, "open");
relm4::new_stateless_action!(SaveAction, WinActions, "save");
relm4::new_stateless_action!(SaveAsAction, WinActions, "save-as");

/// What to do once unsaved changes are dealt with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Then {
    Open,
    Close,
}

pub struct App {
    window: gtk::ApplicationWindow,
    /// Set once the window may really close; its close request asks first.
    may_close: Rc<Cell<bool>>,
    supervisor: Supervisor,
    /// The editor's copy of the document; `None` if the project didn't open.
    copy: Option<WorkingCopy>,
    /// The current stage has replayed the log and matches the copy, so it
    /// can take edits.
    stage_ready: bool,
    stage_view: StageView,
    health: StageHealth,
    smoke: Option<SmokeTest>,
    timeline: Controller<Timeline>,
    /// The root composition's animation and clock; the stage mirrors it.
    playhead: Playhead,
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
    Timeline(TimelineOutput),
    /// Enter: play or pause the timeline.
    TogglePlay,
    Open,
    Save,
    SaveAs,
    /// The user picked where to Save As; then carry on with `Then`.
    SaveTo(PathBuf, Option<Then>),
    /// The user picked a project to open. `discard` drops the current
    /// copy's unsaved edits (they chose Don't Save).
    OpenPath {
        path: PathBuf,
        discard: bool,
    },
    CloseRequested,
    /// Answer to "Save changes?": `None` is Cancel, `Some(true)` Save,
    /// `Some(false)` Don't Save.
    UnsavedAnswer(Then, Option<bool>),
    /// A crashed editor left unsaved edits; `more` other directories also
    /// have some.
    OfferRestore {
        candidate: Candidate,
        more: usize,
    },
    RestoreAnswer(Candidate, RestoreChoice),
    /// Delete a recovery directory: the user confirmed Discard.
    DeleteRecovery(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RestoreChoice {
    Restore,
    Discard,
    NotNow,
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
            connect_close_request[sender, may_close] => move |_| {
                if may_close.get() {
                    glib::Propagation::Proceed
                } else {
                    sender.input(AppMsg::CloseRequested);
                    glib::Propagation::Stop
                }
            },

            #[wrap(Some)]
            set_titlebar = &gtk::HeaderBar {
                pack_start = &gtk::MenuButton {
                    set_icon_name: "open-menu-symbolic",
                    set_tooltip_text: Some("Menu"),
                    set_menu_model: Some(&file_menu),
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
                set_end_child: Some(&timeline_widget),
            },
        }
    }

    fn init(stage_binary: PathBuf, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        relm4::set_global_css(CSS);
        let startup = std::env::var_os(PROJECT_ENV).map(PathBuf::from);
        let (copy, load_error) = match open_copy(startup.as_deref()) {
            Ok(copy) => (Some(copy), None),
            Err(e) => (None, Some(e)),
        };
        let may_close = Rc::new(Cell::new(false));
        let mut model = App {
            window: root.clone(),
            may_close: may_close.clone(),
            supervisor: Supervisor::new(stage_binary),
            copy,
            stage_ready: false,
            stage_view: StageView::default(),
            timeline: Timeline::builder().launch(()).forward(sender.input_sender(), AppMsg::Timeline),
            playhead: Playhead::new(None),
            health: StageHealth::new(Instant::now()),
            smoke: SmokeTest::from_env(),
            status: "Stage starting…".into(),
            banner: None,
            frames_this_second: 0,
            fps: 0,
        };
        let stage_view = &model.stage_view;
        let timeline_widget = model.timeline.widget().clone();
        let file_menu = gio::Menu::new();
        file_menu.append(Some("Open…"), Some("win.open"));
        file_menu.append(Some("Save"), Some("win.save"));
        file_menu.append(Some("Save As…"), Some("win.save-as"));
        let widgets = view_output!();
        root.add_controller(shortcuts(&sender));
        register_file_actions(&root, &sender);
        model.refresh_timeline();

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
        // Once the window is up, offer the newest unsaved work a crashed
        // editor left behind. The rest wait for later launches.
        let candidates = Recovery::root().map(|root| recovery::scan(&root)).unwrap_or_default();
        if let Some(candidate) = candidates.first().cloned() {
            let s = sender.clone();
            let more = candidates.len() - 1;
            glib::idle_add_local_once(move || s.input(AppMsg::OfferRestore { candidate, more }));
        }

        ComponentParts { model, widgets }
    }

    fn shutdown(&mut self, _widgets: &mut Self::Widgets, _output: relm4::Sender<()>) {
        // Unsaved edits stay in the recovery directory.
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
            AppMsg::Timeline(out) => self.on_timeline(out),
            AppMsg::TogglePlay => self.toggle_play(),
            AppMsg::Open => self.check_unsaved(Then::Open, &sender),
            AppMsg::Save => self.save(None, &sender),
            AppMsg::SaveAs => self.save_as(None, &sender),
            AppMsg::SaveTo(picked, then) => match working_copy::save_target(&picked) {
                Ok(dir) => self.save_to(&dir, then, &sender),
                Err(e) => self.alert("Could not save", &e),
            },
            AppMsg::OpenPath { path, discard } => self.open_path(&path, discard, &sender),
            AppMsg::CloseRequested => self.check_unsaved(Then::Close, &sender),
            AppMsg::UnsavedAnswer(_, None) => {}
            AppMsg::UnsavedAnswer(then, Some(true)) => self.save(Some(then), &sender),
            AppMsg::UnsavedAnswer(then, Some(false)) => self.proceed(then, true, &sender),
            AppMsg::OfferRestore { candidate, more } => self.offer_restore(candidate, more, &sender),
            AppMsg::RestoreAnswer(candidate, choice) => self.answer_restore(candidate, choice, &sender),
            AppMsg::DeleteRecovery(dir) => {
                eprintln!("discarding unsaved edits in {}", dir.display());
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    self.alert("Could not discard the unsaved changes", &format!("{}: {e}", dir.display()));
                }
            }
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
        match &self.copy {
            None => "Backstage2D".into(),
            Some(c) if c.is_dirty() => format!("{} • — Backstage2D", c.display_name()),
            Some(c) => format!("{} — Backstage2D", c.display_name()),
        }
    }

    fn alert(&self, message: &str, detail: &str) {
        eprintln!("{message}: {detail}");
        gtk::AlertDialog::builder()
            .message(message)
            .detail(detail)
            .modal(true)
            .build()
            .show(Some(&self.window));
    }

    /// Carries on with `then`, first asking to save if there are unsaved
    /// changes.
    fn check_unsaved(&mut self, then: Then, sender: &ComponentSender<Self>) {
        let Some(copy) = self.copy.as_ref().filter(|c| c.is_dirty()) else {
            return self.proceed(then, false, sender);
        };
        let dialog = gtk::AlertDialog::builder()
            .message(format!("Save changes to “{}”?", copy.display_name()))
            .detail("Your changes will be lost if you don't save them.")
            .buttons(["Cancel", "Don't Save", "Save"])
            .cancel_button(0)
            .default_button(2)
            .modal(true)
            .build();
        let s = sender.clone();
        dialog.choose(Some(&self.window), None::<&gio::Cancellable>, move |answer| {
            let answer = match answer {
                Ok(2) => Some(true),
                Ok(1) => Some(false),
                _ => None,
            };
            s.input(AppMsg::UnsavedAnswer(then, answer));
        });
    }

    /// Unsaved changes are dealt with: saved, or to be dropped if `discard`.
    fn proceed(&mut self, then: Then, discard: bool, sender: &ComponentSender<Self>) {
        match then {
            Then::Open => {
                let dialog = gtk::FileDialog::builder().title("Open Project").modal(true).build();
                let s = sender.clone();
                dialog.select_folder(Some(&self.window), None::<&gio::Cancellable>, move |picked| {
                    if let Some(path) = picked.ok().and_then(|f| f.path()) {
                        s.input(AppMsg::OpenPath { path, discard });
                    }
                });
            }
            Then::Close => {
                if discard && let Some(copy) = self.copy.take() {
                    copy.discard();
                }
                self.may_close.set(true);
                self.window.close();
            }
        }
    }

    /// Replaces the document with the project at `path`, keeping the
    /// running stage: it gets the new document with `Load`.
    fn open_path(&mut self, path: &Path, discard: bool, sender: &ComponentSender<Self>) {
        match open_copy(Some(path)) {
            Ok(copy) => self.replace_copy(copy, discard, sender),
            Err(e) => self.alert("Could not open the project", &format!("{e:#}")),
        }
    }

    /// The animation the timeline shows, in the current document.
    fn shown_animation(&self) -> Option<&Animation> {
        let comp = self.copy.as_ref()?.document().project().root_composition()?;
        comp.animations.get(&self.playhead.animation?)
    }

    fn send_transport(&self) {
        self.supervisor.send(&self.playhead.message(Instant::now()));
    }

    fn show_playhead(&self) {
        let local = self.playhead.local(self.shown_animation(), Instant::now());
        self.timeline.emit(TimelineMsg::SetPlayhead(local));
        self.timeline.emit(TimelineMsg::SetPlaying(self.playhead.is_playing()));
    }

    /// Rebuilds the timeline from the document, after it changed or was
    /// replaced. Falls back to another animation if the shown one is gone.
    fn refresh_timeline(&mut self) {
        let Some(copy) = &self.copy else {
            self.timeline.emit(TimelineMsg::SetModel(TimelineModel::default()));
            return;
        };
        let project = copy.document().project();
        let anim = pick_animation(project, self.playhead.animation);
        let model = TimelineModel::build(project, anim);
        if anim != self.playhead.animation {
            self.playhead.pick(anim);
            self.send_transport();
        }
        self.timeline.emit(TimelineMsg::SetModel(model));
        self.show_playhead();
    }

    fn on_timeline(&mut self, out: TimelineOutput) {
        match out {
            TimelineOutput::Scrub(t) => self.seek(t),
            TimelineOutput::TogglePlay => self.toggle_play(),
            TimelineOutput::PickAnimation(id) => {
                self.playhead.pick(Some(id));
                self.send_transport();
                self.refresh_timeline();
            }
            TimelineOutput::SetSnap(on) => self.set_prefs(|p| p.time_snap = on),
            TimelineOutput::SetGrid(tps) => self.set_prefs(|p| p.time_grid = TimeGrid::new(tps)),
        }
    }

    fn seek(&mut self, t: Time) {
        self.playhead.seek(t);
        self.send_transport();
        self.show_playhead();
    }

    fn toggle_play(&mut self) {
        let anim = self.shown_animation().cloned();
        self.playhead.toggle(anim.as_ref(), Instant::now());
        self.send_transport();
        self.show_playhead();
    }

    /// Editor prefs live in the project, so changing them is an edit.
    fn set_prefs(&mut self, change: impl FnOnce(&mut EditorPrefs)) {
        let Some(copy) = &self.copy else { return };
        let mut prefs = copy.document().project().editor;
        change(&mut prefs);
        self.submit(Entry::Do(Command::SetEditorPrefs(prefs)));
    }

    /// Makes `copy` the document, keeping the running stage if it can: the
    /// stage gets the new document with `Load`. `discard` drops the old
    /// copy's unsaved edits.
    fn replace_copy(&mut self, mut copy: WorkingCopy, discard: bool, sender: &ComponentSender<Self>) {
        self.banner = None;
        self.stage_ready = false;
        let load = copy.load_message();
        self.playhead = Playhead::new(pick_animation(copy.document().project(), None));
        let Some(old) = self.copy.replace(copy) else {
            // Nothing opened at startup, so no stage is running yet.
            self.refresh_timeline();
            return self.start_stage(sender, true);
        };
        self.refresh_timeline();
        // The next `Loaded` must answer this load. If the stage is still
        // loading the old document, it wouldn't, so start a fresh one.
        let restart = old.awaiting_load();
        if discard {
            old.discard();
        } else {
            old.finish();
        }
        if restart {
            self.start_stage(sender, true);
        } else {
            // Commits for the old document may still arrive; the new copy
            // ignores them until the stage reports this load.
            self.supervisor.send(&load);
            self.send_transport();
        }
    }

    fn offer_restore(&mut self, candidate: Candidate, more: usize, sender: &ComponentSender<Self>) {
        if let Some(smoke) = self.smoke.as_mut() {
            let step = smoke.restore_offered();
            if step == Some(SmokeStep::Restore) {
                return self.answer_restore(candidate, RestoreChoice::Restore, sender);
            }
            return self.run_smoke_step(step, sender);
        }
        let edits =
            if candidate.unsaved == 1 { "1 edit".to_owned() } else { format!("{} edits", candidate.unsaved) };
        let when = candidate
            .modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| glib::DateTime::from_unix_local(d.as_secs() as i64).ok())
            .and_then(|t| t.format("%b %e, %H:%M").ok())
            .map_or_else(String::new, |t| format!(" (last edit {t})"));
        let mut detail = format!("Backstage2D closed without saving {edits}{when}.");
        if more > 0 {
            detail.push_str(&format!("\n{more} more set(s) of unsaved changes will be offered next time."));
        }
        let dialog = gtk::AlertDialog::builder()
            .message(format!("Restore unsaved changes to “{}”?", candidate.name))
            .detail(detail)
            .buttons(["Not Now", "Discard", "Restore"])
            .cancel_button(0)
            .default_button(2)
            .modal(true)
            .build();
        let s = sender.clone();
        dialog.choose(Some(&self.window), None::<&gio::Cancellable>, move |answer| {
            let choice = match answer {
                Ok(2) => RestoreChoice::Restore,
                Ok(1) => RestoreChoice::Discard,
                _ => RestoreChoice::NotNow,
            };
            s.input(AppMsg::RestoreAnswer(candidate, choice));
        });
    }

    fn answer_restore(
        &mut self,
        candidate: Candidate,
        choice: RestoreChoice,
        sender: &ComponentSender<Self>,
    ) {
        match choice {
            RestoreChoice::NotNow => {}
            RestoreChoice::Restore => match WorkingCopy::restore(&candidate.dir) {
                Ok(copy) => {
                    eprintln!("restored {} entries from {}", copy.document().seq(), candidate.dir.display());
                    // The fresh startup copy has nothing worth keeping.
                    self.replace_copy(copy, true, sender);
                }
                Err(e) => self.alert("Could not restore the unsaved changes", &e),
            },
            RestoreChoice::Discard => {
                let dialog = gtk::AlertDialog::builder()
                    .message(format!("Discard unsaved changes to “{}”?", candidate.name))
                    .detail("This can't be undone.")
                    .buttons(["Cancel", "Discard"])
                    .cancel_button(0)
                    .default_button(0)
                    .modal(true)
                    .build();
                let s = sender.clone();
                dialog.choose(Some(&self.window), None::<&gio::Cancellable>, move |answer| {
                    if matches!(answer, Ok(1)) {
                        s.input(AppMsg::DeleteRecovery(candidate.dir));
                    }
                });
            }
        }
    }

    /// Save: to the project's path, or Save As if it has none.
    fn save(&mut self, then: Option<Then>, sender: &ComponentSender<Self>) {
        match self.copy.as_ref().map(|c| c.path().map(Path::to_owned)) {
            None => {}
            Some(Some(dir)) => self.save_to(&dir, then, sender),
            Some(None) => self.save_as(then, sender),
        }
    }

    fn save_as(&mut self, then: Option<Then>, sender: &ComponentSender<Self>) {
        let Some(copy) = &self.copy else { return };
        let dialog = gtk::FileDialog::builder()
            .title("Save Project As")
            .initial_name(format!("{}.{}", copy.display_name(), working_copy::PROJECT_EXTENSION))
            .modal(true)
            .build();
        let s = sender.clone();
        dialog.save(Some(&self.window), None::<&gio::Cancellable>, move |picked| {
            // Cancelling also cancels whatever the save was for.
            if let Some(path) = picked.ok().and_then(|f| f.path()) {
                s.input(AppMsg::SaveTo(path, then));
            }
        });
    }

    fn save_to(&mut self, dir: &Path, then: Option<Then>, sender: &ComponentSender<Self>) {
        let Some(copy) = self.copy.as_mut() else { return };
        let result = copy.save_to(dir);
        let saved = match result {
            Ok(()) => true,
            Err(SaveToError::AutosaveFailed(e)) => {
                eprintln!("autosave failed: {e}");
                self.banner = Some(format!("Autosave failed: {e}\nEdits from now on are only in memory."));
                true
            }
            Err(e @ SaveToError::Project(_)) => {
                self.alert("Could not save", &e.to_string());
                false
            }
        };
        if saved {
            eprintln!("saved to {}", dir.display());
            if let Some(then) = then {
                self.proceed(then, false, sender);
            }
        }
        let step = self.smoke.as_mut().and_then(|s| s.saved(saved));
        self.run_smoke_step(step, sender);
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

    fn run_smoke_step(&mut self, step: Option<SmokeStep>, sender: &ComponentSender<Self>) {
        match step {
            None => {}
            Some(SmokeStep::Nudge) => self.update_nudge(10.0, 0.0),
            Some(SmokeStep::Scrub(t)) => {
                // The same path as dragging on the timeline.
                self.on_timeline(TimelineOutput::Scrub(t));
                let step = self.smoke.as_mut().and_then(SmokeTest::scrubbed);
                self.run_smoke_step(step, sender);
            }
            Some(SmokeStep::Undo) => self.submit(Entry::Undo),
            // Carried out by `offer_restore`, which has the candidate.
            Some(SmokeStep::Restore) => {}
            Some(SmokeStep::Save) => match self.copy.as_ref().and_then(|c| c.path().map(Path::to_owned)) {
                Some(dir) => self.save_to(&dir, None, sender),
                None => self.run_smoke_step(Some(SmokeStep::Fail("the project has no path".into())), sender),
            },
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
                if let Some(copy) = self.copy.as_mut() {
                    self.supervisor.send(&copy.load_message());
                }
                self.send_transport();
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
                    if self.playhead.is_playing() {
                        self.show_playhead();
                    }
                    let step = self.smoke.as_mut().and_then(|s| s.frame(self.supervisor.session()));
                    self.run_smoke_step(step, sender);
                }
            }
            StageEvent::Alive => {}
            StageEvent::Log(line) => eprintln!("stage: {line}"),
            StageEvent::Loaded { seq, hash } => {
                let Some(copy) = self.copy.as_mut() else { return };
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
                let loaded = smoke::Loaded {
                    seq,
                    matches: check.is_ok(),
                    dirty: copy.is_dirty(),
                    can_redo: copy.document().can_redo(),
                };
                let step = self.smoke.as_mut().and_then(|s| s.loaded(self.supervisor.session(), loaded));
                self.run_smoke_step(step, sender);
            }
            StageEvent::Committed { seq, entry, .. } => {
                let Some(copy) = self.copy.as_mut() else { return };
                match copy.committed(seq, &entry) {
                    Ok(false) => {} // for the document before an Open
                    Ok(true) => {
                        self.refresh_timeline();
                        let step = self.smoke.as_mut().and_then(|s| s.committed(seq));
                        self.run_smoke_step(step, sender);
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

/// Ctrl+Z / Ctrl+Shift+Z, Enter to play or pause, and the arrow keys for
/// the debug nudge (10 px, or 1 px with Shift). Capture phase, so focused
/// widgets don't swallow the arrows.
fn shortcuts(sender: &ComponentSender<App>) -> gtk::ShortcutController {
    #[derive(Clone, Copy)]
    enum Action {
        Undo,
        Redo,
        Nudge(f32, f32),
        TogglePlay,
    }
    let mut bindings =
        vec![("<Control>z".to_owned(), Action::Undo), ("<Control><Shift>z".to_owned(), Action::Redo)];
    for key in ["Return", "KP_Enter"] {
        bindings.push((key.to_owned(), Action::TogglePlay));
    }
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
                Action::TogglePlay => AppMsg::TogglePlay,
            });
            glib::Propagation::Stop
        });
        controller
            .add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string(&trigger), Some(callback)));
    }
    controller
}

/// Open…, Save, and Save As…, with their accelerators.
fn register_file_actions(window: &gtk::ApplicationWindow, sender: &ComponentSender<App>) {
    let mut group = RelmActionGroup::<WinActions>::new();
    let s = sender.clone();
    group.add_action(RelmAction::<OpenAction>::new_stateless(move |_| s.input(AppMsg::Open)));
    let s = sender.clone();
    group.add_action(RelmAction::<SaveAction>::new_stateless(move |_| s.input(AppMsg::Save)));
    let s = sender.clone();
    group.add_action(RelmAction::<SaveAsAction>::new_stateless(move |_| s.input(AppMsg::SaveAs)));
    group.register_for_widget(window);
    let app = relm4::main_application();
    app.set_accelerators_for_action::<OpenAction>(&["<Control>o"]);
    app.set_accelerators_for_action::<SaveAction>(&["<Control>s"]);
    app.set_accelerators_for_action::<SaveAsAction>(&["<Control><Shift>s"]);
}

/// Opens the project in `dir`, or the built-in sample, with autosave to a
/// fresh recovery directory if one can be made.
fn open_copy(dir: Option<&Path>) -> anyhow::Result<WorkingCopy> {
    use anyhow::Context;
    let project = match dir {
        Some(dir) => backstage_core::load(dir).with_context(|| format!("{}", dir.display()))?,
        None => backstage_core::sample::bounce(),
    };
    let recovery = Recovery::default_dir().and_then(|rdir| Recovery::create(rdir, &project, dir));
    let recovery = match recovery {
        Ok(recovery) => Some(recovery),
        Err(e) => {
            eprintln!("autosave is off: cannot create a recovery directory: {e}");
            None
        }
    };
    let copy = WorkingCopy::new(project, dir.map(Path::to_owned), recovery)?;
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
