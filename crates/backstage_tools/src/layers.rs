//! The Layers panel: the edited composition's node tree (the outliner).
//! It owns structure: selection, expanding and collapsing, and renaming.
//! The timeline shows the same rows ([`crate::tree::visible_rows`]).

use crate::tree::TreeRow;
use backstage_core::NodeId;
use gtk::{gdk, glib, prelude::*};
use relm4::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// Width of the expander slot at the start of each row.
const EXPANDER_W: i32 = 20;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayersModel {
    pub rows: Vec<TreeRow>,
    pub selected: Option<NodeId>,
}

pub struct Layers {
    model: LayersModel,
    /// The row whose name is being edited.
    renaming: Option<usize>,
    /// Set while the list is changed from the model, so its selection
    /// signals aren't taken for clicks.
    quiet: Rc<Cell<bool>>,
}

#[derive(Debug)]
pub enum LayersMsg {
    SetModel(LayersModel),
    /// The list's selected row changed (by a click or the keyboard).
    RowSelected(Option<usize>),
    ToggleExpand(usize),
    /// Double-click or F2: edit the row's name.
    StartRename(Option<usize>),
    CommitRename(String),
    CancelRename,
}

#[derive(Debug)]
pub enum LayersOutput {
    Select(Option<NodeId>),
    ToggleExpand(NodeId),
    Rename { node: NodeId, name: String },
}

pub struct LayersWidgets {
    list: gtk::ListBox,
    /// What the list was last built from.
    built: Option<(Vec<TreeRow>, Option<usize>)>,
}

impl SimpleComponent for Layers {
    type Init = ();
    type Input = LayersMsg;
    type Output = LayersOutput;
    type Root = gtk::Box;
    type Widgets = LayersWidgets;

    fn init_root() -> Self::Root {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("backstage-panel");
        root
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = Layers { model: LayersModel::default(), renaming: None, quiet: Rc::default() };

        let heading = gtk::Label::new(Some("Layers"));
        heading.set_xalign(0.0);
        heading.add_css_class("backstage-panel-title");
        root.append(&heading);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.add_css_class("backstage-layers");
        let (s, quiet) = (sender.clone(), model.quiet.clone());
        list.connect_row_selected(move |_, row| {
            if !quiet.get() {
                s.input(LayersMsg::RowSelected(row.map(|r| r.index() as usize)));
            }
        });
        let keys = gtk::EventControllerKey::new();
        let (s, l) = (sender.clone(), list.clone());
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::F2 {
                s.input(LayersMsg::StartRename(l.selected_row().map(|r| r.index() as usize)));
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        list.add_controller(keys);

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&list));
        root.append(&scroller);

        ComponentParts { model, widgets: LayersWidgets { list, built: None } }
    }

    fn update(&mut self, msg: LayersMsg, sender: ComponentSender<Self>) {
        match msg {
            LayersMsg::SetModel(model) => {
                if model.rows != self.model.rows {
                    self.renaming = None;
                }
                self.model = model;
            }
            LayersMsg::RowSelected(i) => {
                let node = i.and_then(|i| self.model.rows.get(i)).map(|r| r.node);
                if node != self.model.selected {
                    let _ = sender.output(LayersOutput::Select(node));
                }
            }
            LayersMsg::ToggleExpand(i) => {
                if let Some(row) = self.model.rows.get(i) {
                    let _ = sender.output(LayersOutput::ToggleExpand(row.node));
                }
            }
            LayersMsg::StartRename(i) => self.renaming = i.filter(|i| *i < self.model.rows.len()),
            LayersMsg::CommitRename(name) => {
                // Enter, then the focus leaving, both commit: only the first counts.
                if let Some(row) = self.renaming.take().and_then(|i| self.model.rows.get(i)) {
                    let _ = sender.output(LayersOutput::Rename { node: row.node, name });
                }
            }
            LayersMsg::CancelRename => self.renaming = None,
        }
    }

    fn update_view(&self, w: &mut Self::Widgets, sender: ComponentSender<Self>) {
        self.quiet.set(true);
        let key = (self.model.rows.clone(), self.renaming);
        if w.built.as_ref() != Some(&key) {
            while let Some(child) = w.list.first_child() {
                w.list.remove(&child);
            }
            for (i, row) in self.model.rows.iter().enumerate() {
                w.list.append(&row_widget(i, row, self.renaming == Some(i), &sender));
            }
            w.built = Some(key);
        }
        let selected = self.model.selected.and_then(|n| self.model.rows.iter().position(|r| r.node == n));
        match selected.and_then(|i| w.list.row_at_index(i as i32)) {
            Some(row) => w.list.select_row(Some(&row)),
            None => w.list.unselect_all(),
        }
        self.quiet.set(false);
    }
}

/// One row: indentation, an expander (if it has children), and the name,
/// or an entry while renaming.
fn row_widget(i: usize, row: &TreeRow, renaming: bool, sender: &ComponentSender<Layers>) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    line.set_margin_start(4 + row.depth as i32 * 14);
    line.set_margin_end(6);
    line.set_margin_top(1);
    line.set_margin_bottom(1);

    // A fixed-width slot, so names line up whether or not there's an expander.
    let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    slot.set_size_request(EXPANDER_W, -1);
    if row.has_children {
        let icon = if row.expanded { "pan-down-symbolic" } else { "pan-end-symbolic" };
        let expander = gtk::Image::from_icon_name(icon);
        expander.set_halign(gtk::Align::Center);
        expander.set_tooltip_text(Some(if row.expanded { "Collapse" } else { "Expand" }));
        let click = gtk::GestureClick::new();
        let s = sender.clone();
        click.connect_pressed(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            s.input(LayersMsg::ToggleExpand(i));
        });
        expander.add_controller(click);
        slot.append(&expander);
    }
    line.append(&slot);

    if renaming {
        let entry = gtk::Entry::new();
        entry.set_text(&row.name);
        entry.set_hexpand(true);
        let s = sender.clone();
        entry.connect_activate(move |e| s.input(LayersMsg::CommitRename(e.text().to_string())));
        let focus = gtk::EventControllerFocus::new();
        let (s, e) = (sender.clone(), entry.clone());
        focus.connect_leave(move |_| s.input(LayersMsg::CommitRename(e.text().to_string())));
        entry.add_controller(focus);
        let keys = gtk::EventControllerKey::new();
        let s = sender.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                s.input(LayersMsg::CancelRename);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        entry.add_controller(keys);
        line.append(&entry);
        let e = entry.clone();
        glib::idle_add_local_once(move || {
            e.grab_focus();
            e.select_region(0, -1);
        });
    } else {
        let name = gtk::Label::new(Some(&row.name));
        name.set_xalign(0.0);
        name.set_hexpand(true);
        let click = gtk::GestureClick::new();
        let s = sender.clone();
        click.connect_pressed(move |_, n, _, _| {
            if n == 2 {
                s.input(LayersMsg::StartRename(Some(i)));
            }
        });
        name.add_controller(click);
        line.append(&name);
    }
    line
}
