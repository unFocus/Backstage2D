//! The timeline panel: one animation of the root composition at a time, a
//! row per node with its keys, and a playhead the editor owns and sends to
//! the stage (`ToStage::Transport`).
//!
//! The model, layout, and playhead math are plain data, tested without
//! GTK. The [`Timeline`] component draws them with cairo and reports
//! scrubbing and picks to the app.

use crate::tree::{subtree, visible_rows};
use backstage_core::eval::clock::local_time;
use backstage_core::{AnimId, Animation, LoopMode, NodeId, Project, Repeat, Time, TimeGrid};
use backstage_protocol::ToStage;
use gtk::prelude::*;
use relm4::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Instant;

/// Width of the node-name column.
pub const NAME_W: f64 = 160.0;
pub const RULER_H: f64 = 22.0;
pub const ROW_H: f64 = 22.0;
/// Indentation per tree level, and the disclosure triangle's width.
const INDENT: f64 = 12.0;
const EXPANDER_W: f64 = 14.0;
/// Space after the end of the animation.
const PAD_R: f64 = 24.0;
/// Grid lines closer than this are thinned out.
const MIN_TICK_SPACING: f64 = 6.0;
/// Grid choices offered in the toolbar, in ticks per second.
pub const GRID_CHOICES: [u32; 4] = [12, 24, 30, 60];

/// A node's row: its keys in the shown animation.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub node: NodeId,
    pub name: String,
    /// 0 for the root node's children.
    pub depth: usize,
    /// Times with a key on any of the node's tracks, ascending, no
    /// duplicates. A collapsed row has its whole subtree's keys.
    pub keys: Vec<Time>,
    pub has_children: bool,
    pub expanded: bool,
    /// Hidden or locked in the editor (its own flag or an ancestor's).
    pub dim: bool,
}

/// What the timeline shows.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineModel {
    /// The root composition's animations, for the picker.
    pub animations: Vec<(AnimId, String)>,
    /// The one shown, if the composition has any.
    pub animation: Option<AnimId>,
    pub duration: Time,
    pub looping: LoopMode,
    pub rows: Vec<Row>,
    pub grid: TimeGrid,
    pub snap: bool,
    /// The editor's selected node, highlighted.
    pub selected: Option<NodeId>,
}

impl Default for TimelineModel {
    fn default() -> Self {
        Self {
            animations: Vec::new(),
            animation: None,
            duration: Time::ZERO,
            looping: LoopMode::Once,
            rows: Vec::new(),
            grid: TimeGrid::default(),
            snap: true,
            selected: None,
        }
    }
}

impl TimelineModel {
    /// The root composition with `animation` shown, with the same rows as
    /// the Layers panel ([`visible_rows`]).
    pub fn build(project: &Project, animation: Option<AnimId>, expanded: &BTreeSet<NodeId>) -> Self {
        let mut model = TimelineModel {
            grid: project.editor.time_grid,
            snap: project.editor.time_snap,
            ..Self::default()
        };
        let Some(comp) = project.root_composition() else { return model };
        model.animations = comp.animations.iter().map(|(id, a)| (*id, a.name.clone())).collect();
        let anim = animation.and_then(|id| Some((id, comp.animations.get(&id)?)));
        if let Some((id, anim)) = anim {
            model.animation = Some(id);
            model.duration = anim.duration;
            model.looping = anim.looping;
        }

        for row in visible_rows(comp, expanded) {
            // A collapsed row stands for its whole subtree.
            let nodes = if row.has_children && !row.expanded {
                subtree(comp, row.node)
            } else {
                BTreeSet::from([row.node])
            };
            let mut keys: Vec<Time> = anim
                .iter()
                .flat_map(|(_, a)| &a.tracks)
                .filter(|t| nodes.contains(&t.node))
                .flat_map(|t| t.keys.iter().map(|k| k.at))
                .collect();
            keys.sort();
            keys.dedup();
            model.rows.push(Row {
                node: row.node,
                name: row.name,
                depth: row.depth,
                keys,
                has_children: row.has_children,
                expanded: row.expanded,
                dim: row.effective.hidden || row.effective.locked,
            });
        }
        model
    }
}

/// The animation to show: `current` if the root composition still has it,
/// else its default animation, else its first.
pub fn pick_animation(project: &Project, current: Option<AnimId>) -> Option<AnimId> {
    let comp = project.root_composition()?;
    current
        .filter(|id| comp.animations.contains_key(id))
        .or(comp.default_animation.filter(|id| comp.animations.contains_key(id)))
        .or_else(|| comp.animations.keys().next().copied())
}

/// A grid line.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub x: f64,
    pub time: Time,
    /// On a whole second.
    pub major: bool,
    pub label: Option<String>,
}

/// Maps time to x: the animation's duration fits the width after the name
/// column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub width: f64,
    pub duration: Time,
}

impl Layout {
    fn track_w(&self) -> f64 {
        (self.width - NAME_W - PAD_R).max(1.0)
    }

    fn secs(&self) -> f64 {
        self.duration.as_secs_f64()
    }

    pub fn x_of(&self, t: Time) -> f64 {
        if self.secs() <= 0.0 {
            return NAME_W;
        }
        NAME_W + t.as_secs_f64() / self.secs() * self.track_w()
    }

    pub fn time_at(&self, x: f64) -> Time {
        Time::from_secs_f64((x - NAME_W) / self.track_w() * self.secs())
    }

    /// Grid lines from 0 to the end, thinned to stay at least
    /// `MIN_TICK_SPACING` apart. Whole seconds are major; they're labeled
    /// when there's room.
    pub fn ticks(&self, grid: TimeGrid) -> Vec<Tick> {
        let tps = grid.ticks_per_second.max(1) as i64;
        if self.secs() <= 0.0 {
            return Vec::new();
        }
        let px_per_sec = self.track_w() / self.secs();
        let spacing = px_per_sec / tps as f64;
        let stride = (MIN_TICK_SPACING / spacing).ceil().max(1.0) as i64;
        // Label every Nth second so labels don't collide.
        let label_every =
            [1, 2, 5, 10, 30, 60].into_iter().find(|n| *n as f64 * px_per_sec >= 36.0).unwrap_or(120);
        let last = (self.secs() * tps as f64).floor() as i64;
        let mut ticks = Vec::new();
        let mut i = 0;
        while i <= last {
            let time = Time::from_ratio(i, tps);
            let major = i % tps == 0;
            let label = (major && (i / tps) % label_every == 0).then(|| format!("{}s", i / tps));
            ticks.push(Tick { x: self.x_of(time), time, major, label });
            i += stride;
        }
        ticks
    }
}

/// Where a scrub at `x` puts the playhead: snapped to the grid if snapping
/// is on, and within the animation.
/// Where a press at `(x, y)` lands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    /// The ruler or the key area: scrubs.
    Time,
    /// A row's name: selects that row's node.
    Row(usize),
    /// A row's disclosure triangle: expands or collapses it.
    Expander(usize),
    /// The name column below the rows: clears the selection.
    Nothing,
}

pub fn hit(model: &TimelineModel, x: f64, y: f64) -> Hit {
    if x >= NAME_W || y < RULER_H {
        return Hit::Time;
    }
    let i = ((y - RULER_H) / ROW_H).floor() as usize;
    let Some(row) = model.rows.get(i) else { return Hit::Nothing };
    let indent = indent_x(row.depth);
    if row.has_children && x >= indent && x < indent + EXPANDER_W { Hit::Expander(i) } else { Hit::Row(i) }
}

/// Where a row's expander starts; its name follows the expander.
fn indent_x(depth: usize) -> f64 {
    4.0 + depth as f64 * INDENT
}

pub fn scrub_time(model: &TimelineModel, layout: &Layout, x: f64) -> Time {
    let t = layout.time_at(x);
    let t = if model.snap { model.grid.snap(t) } else { t };
    t.clamp(Time::ZERO, model.duration)
}

/// The editor's transport: which animation the root plays, and its clock.
/// The stage mirrors it (`ToStage::Transport`).
#[derive(Debug, Clone, PartialEq)]
pub struct Playhead {
    pub animation: Option<AnimId>,
    state: PlayState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PlayState {
    Paused(Time),
    /// The clock read `from` at `since`.
    Playing {
        from: Time,
        since: Instant,
    },
}

impl Playhead {
    /// Paused at the start of `animation`.
    pub fn new(animation: Option<AnimId>) -> Self {
        Self { animation, state: PlayState::Paused(Time::ZERO) }
    }

    pub fn is_playing(&self) -> bool {
        matches!(self.state, PlayState::Playing { .. })
    }

    /// The root's clock at `now`. It keeps counting past the end; `local`
    /// maps it into the animation.
    pub fn clock(&self, now: Instant) -> Time {
        match self.state {
            PlayState::Paused(t) => t,
            PlayState::Playing { from, since } => {
                let elapsed = now.saturating_duration_since(since);
                from + Time::from_ratio(elapsed.as_nanos() as i64, 1_000_000_000)
            }
        }
    }

    /// The time within `anim` at `now`, looped or ping-ponged exactly as
    /// the stage draws it.
    pub fn local(&self, anim: Option<&Animation>, now: Instant) -> Time {
        match anim {
            Some(anim) => local_time(anim, self.clock(now), Repeat::Natural),
            None => Time::ZERO,
        }
    }

    /// Starts playing from where the playhead is. A play-once animation
    /// sitting at its end starts over.
    pub fn play(&mut self, anim: Option<&Animation>, now: Instant) {
        if self.is_playing() {
            return;
        }
        let mut from = self.local(anim, now);
        if let Some(anim) = anim
            && anim.looping == LoopMode::Once
            && from >= anim.duration
        {
            from = Time::ZERO;
        }
        self.state = PlayState::Playing { from, since: now };
    }

    /// Stops where the playhead is, within the animation.
    pub fn pause(&mut self, anim: Option<&Animation>, now: Instant) {
        self.state = PlayState::Paused(self.local(anim, now));
    }

    pub fn toggle(&mut self, anim: Option<&Animation>, now: Instant) {
        if self.is_playing() { self.pause(anim, now) } else { self.play(anim, now) }
    }

    /// Scrubbing: paused at `t`.
    pub fn seek(&mut self, t: Time) {
        self.state = PlayState::Paused(t);
    }

    /// Shows another animation, paused at its start.
    pub fn pick(&mut self, animation: Option<AnimId>) {
        *self = Self::new(animation);
    }

    /// The stage's copy of this transport.
    pub fn message(&self, now: Instant) -> ToStage {
        ToStage::Transport { animation: self.animation, time: self.clock(now), playing: self.is_playing() }
    }
}

// ---------------------------------------------------------------------------
// GTK

/// What the draw function reads.
#[derive(Default)]
struct Drawn {
    model: TimelineModel,
    playhead: Time,
}

pub struct Timeline {
    drawn: Rc<RefCell<Drawn>>,
    playing: bool,
    /// The current drag started in the ruler or key area.
    scrubbing: bool,
    /// The grid choices in the dropdown (the project's may be extra).
    grids: Vec<u32>,
    /// The drawing area's width at its last draw, for mapping scrubs.
    width: Rc<Cell<f64>>,
}

#[derive(Debug)]
pub enum TimelineMsg {
    SetModel(TimelineModel),
    /// Local time within the shown animation.
    SetPlayhead(Time),
    SetPlaying(bool),
    /// Pointer at x in the drawing area, pressed or dragged.
    DragBegin(f64, f64),
    /// The drag moved to x.
    DragTo(f64),
    PlayClicked,
    AnimationSelected(u32),
    SnapToggled(bool),
    GridSelected(u32),
}

#[derive(Debug)]
pub enum TimelineOutput {
    Scrub(Time),
    Select(Option<NodeId>),
    ToggleExpand(NodeId),
    TogglePlay,
    PickAnimation(AnimId),
    SetSnap(bool),
    SetGrid(u32),
}

pub struct TimelineWidgets {
    area: gtk::DrawingArea,
    play: gtk::Button,
    animations: gtk::DropDown,
    time: gtk::Label,
    snap: gtk::ToggleButton,
    grid: gtk::DropDown,
}

impl SimpleComponent for Timeline {
    type Init = ();
    type Input = TimelineMsg;
    type Output = TimelineOutput;
    type Root = gtk::Box;
    type Widgets = TimelineWidgets;

    fn init_root() -> Self::Root {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("backstage-panel");
        root
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = Timeline {
            drawn: Rc::default(),
            playing: false,
            grids: GRID_CHOICES.to_vec(),
            width: Rc::default(),
            scrubbing: false,
        };

        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.set_margin_start(8);
        bar.set_margin_end(8);
        bar.set_margin_top(4);
        bar.set_margin_bottom(4);
        let title = gtk::Label::new(Some("Timeline"));
        title.add_css_class("backstage-panel-title");
        let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
        play.set_tooltip_text(Some("Play (Enter)"));
        let s = sender.clone();
        play.connect_clicked(move |_| s.input(TimelineMsg::PlayClicked));
        let animations = gtk::DropDown::from_strings(&[]);
        animations.set_tooltip_text(Some("Animation"));
        let s = sender.clone();
        animations.connect_selected_notify(move |d| s.input(TimelineMsg::AnimationSelected(d.selected())));
        let time = gtk::Label::new(None);
        time.add_css_class("backstage-timeline-time");
        time.set_hexpand(true);
        time.set_xalign(0.0);
        let snap = gtk::ToggleButton::with_label("Snap");
        snap.set_tooltip_text(Some("Snap the playhead to the time grid"));
        let s = sender.clone();
        snap.connect_toggled(move |b| s.input(TimelineMsg::SnapToggled(b.is_active())));
        let grid = gtk::DropDown::from_strings(&[]);
        grid.set_tooltip_text(Some("Time grid"));
        let s = sender.clone();
        grid.connect_selected_notify(move |d| s.input(TimelineMsg::GridSelected(d.selected())));
        for w in
            [title.upcast_ref::<gtk::Widget>(), play.upcast_ref(), animations.upcast_ref(), time.upcast_ref()]
        {
            bar.append(w);
        }
        bar.append(&snap);
        bar.append(&grid);

        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        let (drawn, last_width) = (model.drawn.clone(), model.width.clone());
        area.set_draw_func(move |_, cr, width, height| {
            last_width.set(width as f64);
            let _ = draw(cr, &drawn.borrow(), width as f64, height as f64);
        });
        let drag = gtk::GestureDrag::new();
        let s = sender.clone();
        drag.connect_drag_begin(move |_, x, y| s.input(TimelineMsg::DragBegin(x, y)));
        let s = sender.clone();
        drag.connect_drag_update(move |g, dx, _| {
            if let Some((x, _)) = g.start_point() {
                s.input(TimelineMsg::DragTo(x + dx));
            }
        });
        area.add_controller(drag);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_child(Some(&area));
        scroller.set_vexpand(true);

        root.append(&bar);
        root.append(&scroller);
        let widgets = TimelineWidgets { area, play, animations, time, snap, grid };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: TimelineMsg, sender: ComponentSender<Self>) {
        let mut drawn = self.drawn.borrow_mut();
        match msg {
            TimelineMsg::SetModel(model) => {
                if !self.grids.contains(&model.grid.ticks_per_second) {
                    self.grids = GRID_CHOICES.to_vec();
                    self.grids.push(model.grid.ticks_per_second);
                }
                drawn.model = model;
            }
            TimelineMsg::SetPlayhead(t) => drawn.playhead = t,
            TimelineMsg::SetPlaying(playing) => self.playing = playing,
            TimelineMsg::DragBegin(x, y) => {
                self.scrubbing = false;
                match hit(&drawn.model, x, y) {
                    Hit::Time => {
                        self.scrubbing = true;
                        drop(drawn);
                        SimpleComponent::update(self, TimelineMsg::DragTo(x), sender);
                    }
                    Hit::Expander(i) => {
                        let _ = sender.output(TimelineOutput::ToggleExpand(drawn.model.rows[i].node));
                    }
                    Hit::Row(i) => {
                        let _ = sender.output(TimelineOutput::Select(Some(drawn.model.rows[i].node)));
                    }
                    Hit::Nothing => {
                        let _ = sender.output(TimelineOutput::Select(None));
                    }
                }
            }
            TimelineMsg::DragTo(x) => {
                let width = self.width.get();
                let layout = Layout { width, duration: drawn.model.duration };
                if self.scrubbing && drawn.model.animation.is_some() {
                    let t = scrub_time(&drawn.model, &layout, x);
                    drawn.playhead = t;
                    self.playing = false;
                    let _ = sender.output(TimelineOutput::Scrub(t));
                }
            }
            TimelineMsg::PlayClicked => {
                let _ = sender.output(TimelineOutput::TogglePlay);
            }
            TimelineMsg::AnimationSelected(i) => {
                if let Some((id, _)) = drawn.model.animations.get(i as usize)
                    && drawn.model.animation != Some(*id)
                {
                    let _ = sender.output(TimelineOutput::PickAnimation(*id));
                }
            }
            TimelineMsg::SnapToggled(on) => {
                if on != drawn.model.snap {
                    let _ = sender.output(TimelineOutput::SetSnap(on));
                }
            }
            TimelineMsg::GridSelected(i) => {
                if let Some(&tps) = self.grids.get(i as usize)
                    && tps != drawn.model.grid.ticks_per_second
                {
                    let _ = sender.output(TimelineOutput::SetGrid(tps));
                }
            }
        }
    }

    fn update_view(&self, widgets: &mut Self::Widgets, _sender: ComponentSender<Self>) {
        let drawn = self.drawn.borrow();
        let model = &drawn.model;

        widgets.play.set_icon_name(if self.playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        });
        widgets.play.set_tooltip_text(Some(if self.playing { "Pause (Enter)" } else { "Play (Enter)" }));
        widgets.play.set_sensitive(model.animation.is_some());

        // Rebuilding the lists re-selects, which the handlers above ignore
        // when nothing changed.
        let names: Vec<&str> = model.animations.iter().map(|(_, n)| n.as_str()).collect();
        if list_strings(&widgets.animations) != names {
            widgets.animations.set_model(Some(&gtk::StringList::new(&names)));
        }
        let selected = model.animations.iter().position(|(id, _)| Some(*id) == model.animation);
        widgets.animations.set_selected(selected.map_or(gtk::INVALID_LIST_POSITION, |i| i as u32));
        widgets.animations.set_sensitive(!names.is_empty());

        let grid_names: Vec<String> = self.grids.iter().map(|t| format!("{t}/s")).collect();
        let grid_refs: Vec<&str> = grid_names.iter().map(String::as_str).collect();
        if list_strings(&widgets.grid) != grid_refs {
            widgets.grid.set_model(Some(&gtk::StringList::new(&grid_refs)));
        }
        if let Some(i) = self.grids.iter().position(|&t| t == model.grid.ticks_per_second) {
            widgets.grid.set_selected(i as u32);
        }
        widgets.snap.set_active(model.snap);

        widgets.time.set_label(&if model.animation.is_some() {
            format!("{:.2} s / {:.2} s", drawn.playhead.as_secs_f64(), model.duration.as_secs_f64())
        } else {
            "No animations".to_owned()
        });
        widgets.area.set_content_height((RULER_H + ROW_H * model.rows.len() as f64) as i32 + 8);
        widgets.area.queue_draw();
    }
}

fn list_strings(dropdown: &gtk::DropDown) -> Vec<String> {
    let Some(list) = dropdown.model().and_downcast::<gtk::StringList>() else { return Vec::new() };
    (0..list.n_items()).filter_map(|i| list.string(i)).map(|s| s.to_string()).collect()
}

fn draw(cr: &gtk::cairo::Context, drawn: &Drawn, width: f64, height: f64) -> Result<(), gtk::cairo::Error> {
    let model = &drawn.model;
    let layout = Layout { width, duration: model.duration };
    cr.set_source_rgb(0.16, 0.16, 0.18);
    cr.paint()?;
    cr.set_font_size(11.0);
    cr.set_line_width(1.0);

    // Grid: faint ticks, brighter whole seconds, labels on the ruler.
    for tick in layout.ticks(model.grid) {
        let x = tick.x.floor() + 0.5;
        if tick.major {
            cr.set_source_rgb(0.34, 0.34, 0.38);
        } else {
            cr.set_source_rgb(0.23, 0.23, 0.26);
        }
        cr.move_to(x, if tick.major { RULER_H - 8.0 } else { RULER_H - 4.0 });
        cr.line_to(x, height);
        cr.stroke()?;
        if let Some(label) = &tick.label {
            cr.set_source_rgb(0.65, 0.65, 0.7);
            cr.move_to(x + 3.0, RULER_H - 9.0);
            cr.show_text(label)?;
        }
    }
    // End of the animation.
    if model.animation.is_some() {
        let x = layout.x_of(model.duration).floor() + 0.5;
        cr.set_source_rgb(0.5, 0.5, 0.56);
        cr.move_to(x, 0.0);
        cr.line_to(x, height);
        cr.stroke()?;
    }

    // Rows: name column, separators, and key marks.
    cr.set_source_rgb(0.19, 0.19, 0.21);
    cr.rectangle(0.0, 0.0, NAME_W, height);
    cr.fill()?;
    for (i, row) in model.rows.iter().enumerate() {
        let y = RULER_H + i as f64 * ROW_H;
        if model.selected == Some(row.node) {
            cr.set_source_rgba(0.25, 0.45, 0.8, 0.45);
            cr.rectangle(0.0, y, width, ROW_H);
            cr.fill()?;
        }
        cr.set_source_rgb(0.28, 0.28, 0.31);
        cr.move_to(0.0, (y + ROW_H).floor() + 0.5);
        cr.line_to(width, (y + ROW_H).floor() + 0.5);
        cr.stroke()?;
        let indent = indent_x(row.depth);
        if row.has_children {
            // ▸ collapsed, ▾ expanded.
            let (cx, cy) = (indent + EXPANDER_W / 2.0, y + ROW_H / 2.0);
            cr.set_source_rgb(0.7, 0.7, 0.75);
            if row.expanded {
                cr.move_to(cx - 4.0, cy - 2.0);
                cr.line_to(cx + 4.0, cy - 2.0);
                cr.line_to(cx, cy + 3.0);
            } else {
                cr.move_to(cx - 2.0, cy - 4.0);
                cr.line_to(cx + 3.0, cy);
                cr.line_to(cx - 2.0, cy + 4.0);
            }
            cr.close_path();
            cr.fill()?;
        }
        if row.dim {
            cr.set_source_rgb(0.5, 0.5, 0.54);
        } else {
            cr.set_source_rgb(0.85, 0.85, 0.88);
        }
        cr.move_to(indent + EXPANDER_W, y + ROW_H - 7.0);
        cr.show_text(&row.name)?;
        // A collapsed row's keys stand for its subtree: dimmer.
        let summary = row.has_children && !row.expanded;

        if let (Some(first), Some(last)) = (row.keys.first(), row.keys.last()) {
            cr.set_source_rgba(0.35, 0.55, 0.85, 0.35);
            cr.rectangle(layout.x_of(*first), y + 4.0, layout.x_of(*last) - layout.x_of(*first), ROW_H - 8.0);
            cr.fill()?;
        }
        if summary {
            cr.set_source_rgb(0.6, 0.6, 0.66);
        } else {
            cr.set_source_rgb(0.9, 0.9, 0.95);
        }
        for &key in &row.keys {
            let (x, cy, r) = (layout.x_of(key), y + ROW_H / 2.0, 4.0);
            cr.move_to(x, cy - r);
            cr.line_to(x + r, cy);
            cr.line_to(x, cy + r);
            cr.line_to(x - r, cy);
            cr.close_path();
            cr.fill()?;
        }
    }
    cr.set_source_rgb(0.28, 0.28, 0.31);
    cr.move_to(0.0, RULER_H - 0.5);
    cr.line_to(width, RULER_H - 0.5);
    cr.stroke()?;

    // Playhead.
    if model.animation.is_some() {
        let x = layout.x_of(drawn.playhead).floor() + 0.5;
        cr.set_source_rgb(0.95, 0.25, 0.3);
        cr.rectangle(x - 4.0, 2.0, 8.0, RULER_H - 6.0);
        cr.fill()?;
        cr.move_to(x, RULER_H - 4.0);
        cr.line_to(x, height);
        cr.stroke()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use backstage_core::sample::{self, ids::*};
    use std::time::Duration;

    /// Everything expanded, as a document starts.
    fn build(project: &Project, anim: Option<AnimId>) -> TimelineModel {
        TimelineModel::build(project, anim, &crate::tree::expandable(project.root_composition().unwrap()))
    }

    fn secs(num: i64, den: i64) -> Time {
        Time::from_ratio(num, den)
    }

    #[test]
    fn rows_follow_the_tree_with_the_animations_keys() {
        let project = sample::bounce();
        let model = build(&project, Some(STAGE_MAIN));
        let main = &project.compositions[&STAGE].animations[&STAGE_MAIN];
        assert_eq!(model.animations, [(STAGE_MAIN, "main".to_owned())]);
        assert_eq!(
            (model.animation, model.duration, model.looping),
            (Some(STAGE_MAIN), main.duration, main.looping)
        );

        let root = project.compositions[&STAGE].root_node().unwrap();
        let ids: Vec<_> = model.rows.iter().map(|r| r.node).collect();
        assert_eq!(ids, root.children, "top-level nodes in child order");
        assert!(model.rows.iter().all(|r| r.depth == 0));
        for row in &model.rows {
            let mut expected: Vec<Time> = main
                .tracks
                .iter()
                .filter(|t| t.node == row.node)
                .flat_map(|t| t.keys.iter().map(|k| k.at))
                .collect();
            expected.sort();
            expected.dedup();
            assert_eq!(row.keys, expected, "{}", row.name);
        }
        assert!(model.rows.iter().any(|r| !r.keys.is_empty()), "the sample animates something");
        assert!(model.rows.iter().any(|r| r.keys.is_empty()), "and leaves something alone");
    }

    #[test]
    fn nested_nodes_are_indented_and_missing_animations_show_no_keys() {
        let mut project = sample::bounce();
        project.root = BALL;
        let model = build(&project, Some(BALL_SQUASH));
        assert_eq!(model.animations.len(), 2);
        assert_eq!(model.rows.iter().map(|r| (r.node, r.depth)).collect::<Vec<_>>(), [(BALL_BODY, 0)]);
        let squash = &project.compositions[&BALL].animations[&BALL_SQUASH];
        assert_eq!(model.duration, squash.duration);

        let model = build(&project, Some(STAGE_MAIN));
        assert_eq!(model.animation, None, "not an animation of this composition");
        assert!(model.rows.iter().all(|r| r.keys.is_empty()));

        let mut project = sample::bounce();
        let stage = project.compositions.get_mut(&STAGE).unwrap();
        let ground = stage.nodes.get_mut(&GROUND).unwrap();
        ground.children = vec![EYES];
        stage.nodes.get_mut(&STAGE_ROOT).unwrap().children.retain(|&n| n != EYES);
        let model = build(&project, Some(STAGE_MAIN));
        let depths: Vec<_> = model.rows.iter().map(|r| (r.node, r.depth)).collect();
        assert_eq!(&depths[..2], [(GROUND, 0), (EYES, 1)]);
    }

    #[test]
    fn the_picked_animation_falls_back_to_the_default_then_the_first() {
        let mut project = sample::bounce();
        assert_eq!(pick_animation(&project, Some(STAGE_MAIN)), Some(STAGE_MAIN));
        assert_eq!(pick_animation(&project, Some(BALL_BOUNCE)), Some(STAGE_MAIN), "not the root's");
        assert_eq!(pick_animation(&project, None), Some(STAGE_MAIN));
        project.compositions.get_mut(&STAGE).unwrap().default_animation = None;
        assert_eq!(pick_animation(&project, None), Some(STAGE_MAIN), "first");
        project.compositions.get_mut(&STAGE).unwrap().animations.clear();
        assert_eq!(pick_animation(&project, None), None);
    }

    #[test]
    fn collapsed_rows_summarize_their_subtree_and_the_timeline_mirrors_the_tree() {
        use crate::tree::tests::{GROUP, grouped};
        let project = grouped();
        let comp = &project.compositions[&STAGE];
        let open = TimelineModel::build(&project, Some(STAGE_MAIN), &BTreeSet::from([GROUP]));
        let rows = visible_rows(comp, &BTreeSet::from([GROUP]));
        assert_eq!(
            open.rows.iter().map(|r| (r.node, r.depth)).collect::<Vec<_>>(),
            rows.iter().map(|r| (r.node, r.depth)).collect::<Vec<_>>()
        );
        assert!(open.rows[0].keys.is_empty(), "the group itself has no keys");

        // Key the ground, then collapse its group: the group's row shows it.
        let mut project = project;
        let anim = project.compositions.get_mut(&STAGE).unwrap().animations.get_mut(&STAGE_MAIN).unwrap();
        anim.tracks.push(backstage_core::Track {
            node: GROUND,
            property: backstage_core::Property::Y,
            keys: vec![backstage_core::Key::new(
                secs(1, 2),
                backstage_core::Value::Number(1.0),
                Default::default(),
            )],
        });
        let closed = TimelineModel::build(&project, Some(STAGE_MAIN), &BTreeSet::new());
        assert_eq!(closed.rows.len(), 3);
        assert_eq!((closed.rows[0].node, closed.rows[0].expanded), (GROUP, false));
        assert_eq!(closed.rows[0].keys, [secs(1, 2)]);
    }

    #[test]
    fn presses_select_rows_clear_the_selection_or_scrub() {
        let project = sample::bounce();
        let model = build(&project, Some(STAGE_MAIN));
        let n = model.rows.len();
        assert_eq!(hit(&model, 10.0, RULER_H + 1.0), Hit::Row(0));
        assert_eq!(hit(&model, 10.0, RULER_H + ROW_H * (n as f64 - 0.5)), Hit::Row(n - 1));
        assert_eq!(hit(&model, 10.0, RULER_H + ROW_H * n as f64 + 1.0), Hit::Nothing);
        assert_eq!(hit(&model, 10.0, 5.0), Hit::Time, "the ruler");
        assert_eq!(hit(&model, NAME_W + 1.0, RULER_H + 1.0), Hit::Time, "the key area");

        let grouped = build(&crate::tree::tests::grouped(), Some(STAGE_MAIN));
        assert_eq!(hit(&grouped, indent_x(0) + 2.0, RULER_H + 1.0), Hit::Expander(0));
        assert_eq!(hit(&grouped, indent_x(0) + EXPANDER_W + 2.0, RULER_H + 1.0), Hit::Row(0));
        assert_eq!(hit(&grouped, indent_x(1) + 2.0, RULER_H + ROW_H + 1.0), Hit::Row(1), "no children");
    }

    #[test]
    fn x_and_time_round_trip() {
        let layout = Layout { width: 972.0, duration: Time::from_secs(4) };
        assert_eq!(layout.x_of(Time::ZERO), NAME_W);
        assert_eq!(layout.x_of(Time::from_secs(4)), 972.0 - PAD_R);
        for t in [secs(0, 1), secs(1, 3), secs(5, 2), secs(4, 1)] {
            let back = layout.time_at(layout.x_of(t));
            assert!((back - t).as_secs_f64().abs() < 1e-6, "{t} -> {back}");
        }
    }

    #[test]
    fn scrubbing_snaps_to_the_grid_and_stays_in_the_animation() {
        let project = sample::bounce();
        let mut model = build(&project, Some(STAGE_MAIN));
        let layout = Layout { width: NAME_W + 400.0 + PAD_R, duration: Time::from_secs(4) }; // 100 px/s
        model.grid = TimeGrid::new(60);
        assert_eq!(scrub_time(&model, &layout, NAME_W + 50.4), secs(30, 60));
        model.grid = TimeGrid::new(24);
        assert_eq!(scrub_time(&model, &layout, NAME_W + 50.4), secs(12, 24));
        model.snap = false;
        let t = scrub_time(&model, &layout, NAME_W + 50.4);
        assert!((t.as_secs_f64() - 0.504).abs() < 1e-6, "{t}");
        model.snap = true;
        assert_eq!(scrub_time(&model, &layout, 0.0), Time::ZERO);
        assert_eq!(scrub_time(&model, &layout, 5000.0), Time::from_secs(4));
    }

    #[test]
    fn ticks_are_thinned_and_whole_seconds_are_labeled() {
        for (width, tps) in [(400.0, 60), (2000.0, 60), (400.0, 12), (180.0, 24)] {
            let layout = Layout { width, duration: Time::from_secs(4) };
            let ticks = layout.ticks(TimeGrid::new(tps));
            assert!(ticks.windows(2).all(|w| w[1].x - w[0].x >= MIN_TICK_SPACING - 1e-9), "{width} {tps}");
            assert_eq!(ticks[0].label.as_deref(), Some("0s"));
            assert!(ticks.iter().all(|t| t.label.is_none() || t.major));
        }
        let wide = Layout { width: 2000.0, duration: Time::from_secs(4) }.ticks(TimeGrid::new(12));
        assert_eq!(wide.len(), 49, "every tick fits");
        assert_eq!(wide.iter().filter(|t| t.label.is_some()).count(), 5, "0s..4s");
    }

    #[test]
    fn the_playhead_pauses_plays_and_loops_like_the_stage() {
        let project = sample::bounce();
        let main = &project.compositions[&STAGE].animations[&STAGE_MAIN]; // 4 s ping-pong
        let t0 = Instant::now();
        let mut playhead = Playhead::new(Some(STAGE_MAIN));
        assert_eq!(playhead.clock(t0 + Duration::from_secs(3)), Time::ZERO, "starts paused at 0");

        playhead.seek(Time::from_secs(1));
        playhead.play(Some(main), t0);
        assert!(playhead.is_playing());
        let later = t0 + Duration::from_secs(4);
        assert_eq!(playhead.clock(later), Time::from_secs(5));
        assert_eq!(playhead.local(Some(main), later), Time::from_secs(3), "ping-pong on the way back");
        assert_eq!(
            playhead.message(later),
            ToStage::Transport { animation: Some(STAGE_MAIN), time: Time::from_secs(5), playing: true }
        );

        playhead.pause(Some(main), later);
        assert_eq!(
            playhead.clock(later + Duration::from_secs(9)),
            Time::from_secs(3),
            "paused within the animation"
        );
        playhead.toggle(Some(main), later);
        assert!(playhead.is_playing());
        playhead.seek(Time::from_secs(2));
        assert!(!playhead.is_playing(), "scrubbing pauses");
        playhead.pick(Some(BALL_BOUNCE));
        assert_eq!(playhead, Playhead::new(Some(BALL_BOUNCE)));
    }

    #[test]
    fn a_play_once_animation_at_its_end_starts_over() {
        let project = sample::bounce();
        let squash = &project.compositions[&BALL].animations[&BALL_SQUASH];
        let t0 = Instant::now();
        let mut playhead = Playhead::new(Some(BALL_SQUASH));
        playhead.seek(squash.duration);
        playhead.play(Some(squash), t0);
        assert_eq!(playhead.clock(t0), Time::ZERO);
    }
}
