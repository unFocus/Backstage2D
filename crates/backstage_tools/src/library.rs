//! The Library panel: the project's compositions and assets. Double-click
//! (or Enter on) a composition to edit it; the one being edited is
//! highlighted. Assets are listed but can't be opened yet.

use backstage_core::{AssetId, AssetKind, CompId, Project};
use gtk::prelude::*;
use relm4::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// What a library row stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Composition(CompId),
    Asset(AssetId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryRow {
    pub item: Item,
    pub name: String,
    /// "Stage", "2 animations", "Bitmap", …
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LibraryModel {
    /// Compositions, then assets, each sorted by name.
    pub rows: Vec<LibraryRow>,
    /// The composition being edited.
    pub editing: Option<CompId>,
}

impl LibraryModel {
    pub fn build(project: &Project, editing: CompId) -> Self {
        let by_name = |a: &LibraryRow, b: &LibraryRow| a.name.to_lowercase().cmp(&b.name.to_lowercase());
        let mut comps: Vec<_> = project
            .compositions
            .values()
            .map(|c| LibraryRow {
                item: Item::Composition(c.id),
                name: c.name.clone(),
                detail: if c.id == project.root {
                    "Stage".to_owned()
                } else {
                    match c.animations.len() {
                        1 => "1 animation".to_owned(),
                        n => format!("{n} animations"),
                    }
                },
            })
            .collect();
        comps.sort_by(by_name);
        let mut assets: Vec<_> = project
            .assets
            .iter()
            .map(|(id, a)| LibraryRow {
                item: Item::Asset(*id),
                name: a.name.clone(),
                detail: match a.kind {
                    AssetKind::Bitmap => "Bitmap",
                    AssetKind::Font => "Font",
                    AssetKind::Audio => "Audio",
                }
                .to_owned(),
            })
            .collect();
        assets.sort_by(by_name);
        comps.extend(assets);
        LibraryModel { rows: comps, editing: Some(editing) }
    }
}

/// What activating a row edits: compositions only.
pub fn edit_target(row: &LibraryRow) -> Option<CompId> {
    match row.item {
        Item::Composition(c) => Some(c),
        Item::Asset(_) => None,
    }
}

pub struct Library {
    model: LibraryModel,
    /// Set while the list is changed from the model, so its selection
    /// signals aren't taken for clicks.
    quiet: Rc<Cell<bool>>,
}

#[derive(Debug)]
pub enum LibraryMsg {
    SetModel(LibraryModel),
    /// A row was double-clicked or activated with Enter.
    Activated(usize),
}

#[derive(Debug)]
pub enum LibraryOutput {
    Edit(CompId),
}

pub struct LibraryWidgets {
    list: gtk::ListBox,
    built: Option<Vec<LibraryRow>>,
}

impl SimpleComponent for Library {
    type Init = ();
    type Input = LibraryMsg;
    type Output = LibraryOutput;
    type Root = gtk::Box;
    type Widgets = LibraryWidgets;

    fn init_root() -> Self::Root {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("backstage-panel");
        root
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = Library { model: LibraryModel::default(), quiet: Rc::default() };
        let heading = gtk::Label::new(Some("Library"));
        heading.set_xalign(0.0);
        heading.add_css_class("backstage-panel-title");
        root.append(&heading);

        let list = gtk::ListBox::new();
        list.set_activate_on_single_click(false);
        let (s, quiet) = (sender.clone(), model.quiet.clone());
        list.connect_row_activated(move |_, row| {
            if !quiet.get() {
                s.input(LibraryMsg::Activated(row.index() as usize));
            }
        });
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&list));
        root.append(&scroller);
        ComponentParts { model, widgets: LibraryWidgets { list, built: None } }
    }

    fn update(&mut self, msg: LibraryMsg, sender: ComponentSender<Self>) {
        match msg {
            LibraryMsg::SetModel(model) => self.model = model,
            LibraryMsg::Activated(i) => {
                if let Some(comp) = self.model.rows.get(i).and_then(edit_target) {
                    let _ = sender.output(LibraryOutput::Edit(comp));
                }
            }
        }
    }

    fn update_view(&self, w: &mut Self::Widgets, _sender: ComponentSender<Self>) {
        self.quiet.set(true);
        if w.built.as_ref() != Some(&self.model.rows) {
            while let Some(child) = w.list.first_child() {
                w.list.remove(&child);
            }
            for row in &self.model.rows {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                line.set_margin_start(8);
                line.set_margin_end(8);
                line.set_margin_top(4);
                line.set_margin_bottom(4);
                let name = gtk::Label::new(Some(&row.name));
                name.set_hexpand(true);
                name.set_xalign(0.0);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                let detail = gtk::Label::new(Some(&row.detail));
                detail.add_css_class("dim-label");
                line.append(&name);
                line.append(&detail);
                if edit_target(row).is_some() {
                    line.set_tooltip_text(Some("Double-click to edit"));
                }
                w.list.append(&line);
            }
            w.built = Some(self.model.rows.clone());
        }
        let editing = self
            .model
            .rows
            .iter()
            .position(|r| matches!(r.item, Item::Composition(c) if Some(c) == self.model.editing));
        match editing.and_then(|i| w.list.row_at_index(i as i32)) {
            Some(row) => w.list.select_row(Some(&row)),
            None => w.list.unselect_all(),
        }
        self.quiet.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backstage_core::sample::{self, ids::*};
    use backstage_core::{Asset, AssetId};

    #[test]
    fn compositions_then_assets_sorted_with_the_stage_marked() {
        let mut project = sample::bounce();
        project.assets.insert(
            AssetId::from_raw(2),
            Asset { name: "zap.wav".into(), path: "assets/zap.wav".into(), kind: AssetKind::Audio },
        );
        project.assets.insert(
            AssetId::from_raw(1),
            Asset { name: "Logo.png".into(), path: "assets/logo.png".into(), kind: AssetKind::Bitmap },
        );
        let model = LibraryModel::build(&project, BALL);
        let rows: Vec<_> = model.rows.iter().map(|r| (r.name.as_str(), r.detail.as_str())).collect();
        assert_eq!(
            rows,
            [
                ("Ball", "2 animations"),
                ("Blinker", "1 animation"),
                ("Stage", "Stage"),
                ("Logo.png", "Bitmap"),
                ("zap.wav", "Audio")
            ]
        );
        assert_eq!(model.editing, Some(BALL));
    }

    #[test]
    fn only_compositions_can_be_edited() {
        let project = sample::bounce();
        let model = LibraryModel::build(&project, STAGE);
        let comps: Vec<_> = model.rows.iter().filter_map(edit_target).collect();
        assert_eq!(comps, [BALL, BLINKER, STAGE]);
        let asset =
            LibraryRow { item: Item::Asset(AssetId::from_raw(1)), name: "a".into(), detail: String::new() };
        assert_eq!(edit_target(&asset), None);
    }
}
