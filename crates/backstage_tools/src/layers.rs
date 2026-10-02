//! The Layers panel: the edited composition's node tree (the outliner).
//! It owns structure: selection, expanding and collapsing, renaming, and
//! the editor-only hide/lock/outline toggles. The timeline shows the same
//! rows ([`crate::tree::visible_rows`]).

use crate::tree::TreeRow;
use backstage_core::{Command, CompId, Entry, NodeId, Project, editor};
use gtk::{gdk, glib, prelude::*};
use relm4::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// Width of the expander slot at the start of each row.
const EXPANDER_W: i32 = 20;

/// One of a node's editor-only flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    Hidden,
    Locked,
    Outline,
}

/// A `SetNodeFlags` turning `flag` on or off for `node`, or `None` if the
/// node doesn't exist.
pub fn flags_edit(project: &Project, comp: CompId, node: NodeId, flag: Flag) -> Option<Entry> {
    let mut flags = project.compositions.get(&comp)?.nodes.get(&node)?.editor;
    match flag {
        Flag::Hidden => flags.hidden = !flags.hidden,
        Flag::Locked => flags.locked = !flags.locked,
        Flag::Outline => flags.outline = !flags.outline,
    }
    Some(Entry::Do(Command::SetNodeFlags { comp, node, flags }))
}

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
    ToggleFlag(usize, Flag),
    /// Double-click on an instance: edit its composition.
    Enter(usize),
    /// Double-click or F2: edit the row's name.
    StartRename(Option<usize>),
    CommitRename(String),
    CancelRename,
}

#[derive(Debug)]
pub enum LayersOutput {
    Select(Option<NodeId>),
    ToggleExpand(NodeId),
    ToggleFlag(NodeId, Flag),
    Rename {
        node: NodeId,
        name: String,
    },
    /// Edit this composition (an instance row was double-clicked).
    Enter(CompId),
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
            LayersMsg::ToggleFlag(i, flag) => {
                if let Some(row) = self.model.rows.get(i) {
                    let _ = sender.output(LayersOutput::ToggleFlag(row.node, flag));
                }
            }
            LayersMsg::Enter(i) => {
                if let Some(comp) = self.model.rows.get(i).and_then(|r| r.enters) {
                    let _ = sender.output(LayersOutput::Enter(comp));
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
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        if row.effective.hidden {
            name.add_css_class("dim-label");
        }
        // Double-click: an instance enters its composition; anything else
        // is renamed (F2 renames instances too).
        let enters = row.enters.is_some();
        if enters {
            name.set_tooltip_text(Some("Double-click to edit its composition (F2 to rename)"));
        }
        let click = gtk::GestureClick::new();
        let s = sender.clone();
        click.connect_pressed(move |_, n, _, _| {
            if n == 2 {
                s.input(if enters { LayersMsg::Enter(i) } else { LayersMsg::StartRename(Some(i)) });
            }
        });
        name.add_controller(click);
        line.append(&name);
    }
    for flag in [Flag::Hidden, Flag::Locked, Flag::Outline] {
        line.append(&flag_toggle(i, row, flag, sender));
    }
    line
}

/// A small flat button showing one flag. On if the node's own flag is set;
/// faded if only an ancestor's is.
fn flag_toggle(i: usize, row: &TreeRow, flag: Flag, sender: &ComponentSender<Layers>) -> gtk::Button {
    let (own, effective) = match flag {
        Flag::Hidden => (row.flags.hidden, row.effective.hidden),
        Flag::Locked => (row.flags.locked, row.effective.locked),
        Flag::Outline => (row.flags.outline, row.effective.outline),
    };
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("backstage-flag");
    match flag {
        Flag::Hidden => {
            button.set_icon_name(if effective { "view-conceal-symbolic" } else { "view-reveal-symbolic" });
            button.set_tooltip_text(Some(if own { "Show in the editor" } else { "Hide in the editor" }));
        }
        Flag::Locked => {
            button.set_icon_name(if effective {
                "changes-prevent-symbolic"
            } else {
                "changes-allow-symbolic"
            });
            button.set_tooltip_text(Some(if own { "Unlock" } else { "Lock" }));
        }
        Flag::Outline => {
            // A square in the node's outline colour: filled when outlined.
            let swatch = gtk::DrawingArea::new();
            swatch.set_content_width(12);
            swatch.set_content_height(12);
            let [r, g, b, _] = editor::outline_color(row.node).to_rgba8();
            swatch.set_draw_func(move |_, cr, w, h| {
                cr.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
                cr.rectangle(1.5, 1.5, w as f64 - 3.0, h as f64 - 3.0);
                if effective {
                    let _ = cr.fill();
                } else {
                    cr.set_line_width(1.0);
                    let _ = cr.stroke();
                }
            });
            button.set_child(Some(&swatch));
            button.set_tooltip_text(Some(if own { "Draw normally" } else { "Show as outlines" }));
        }
    }
    // Off, or only inherited: faded, so the node's own settings stand out.
    button.set_opacity(if own {
        1.0
    } else if effective {
        0.55
    } else {
        0.3
    });
    let s = sender.clone();
    button.connect_clicked(move |_| s.input(LayersMsg::ToggleFlag(i, flag)));
    button
}

#[cfg(test)]
mod tests {
    use super::*;
    use backstage_core::sample::{self, ids::*};

    #[test]
    fn flag_edits_toggle_one_flag_and_undo_exactly() {
        let project = sample::bounce();
        for flag in [Flag::Hidden, Flag::Locked, Flag::Outline] {
            let Some(Entry::Do(cmd)) = flags_edit(&project, STAGE, GROUND, flag) else { panic!() };
            let mut edited = project.clone();
            let inverse = cmd.apply(&mut edited).unwrap();
            let flags = edited.compositions[&STAGE].nodes[&GROUND].editor;
            assert_eq!(
                [flags.hidden, flags.locked, flags.outline].iter().filter(|f| **f).count(),
                1,
                "{flag:?}"
            );
            let Some(Entry::Do(back)) = flags_edit(&edited, STAGE, GROUND, flag) else { panic!() };
            let mut again = edited.clone();
            back.apply(&mut again).unwrap();
            assert_eq!(again, project, "toggling twice turns it off again");
            inverse.apply(&mut edited).unwrap();
            assert_eq!(edited, project);
        }
        assert_eq!(flags_edit(&project, STAGE, NodeId::from_raw(1), Flag::Hidden), None);
    }
}
