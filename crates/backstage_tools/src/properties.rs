//! The properties panel: the selected node's rest values, or the document
//! settings when nothing is selected. Every change goes out as an entry
//! (`SetRest`, `RenameNode`, `SetSettings`), so it's undoable and autosaved
//! like any edit.
//!
//! Rest values are what a node shows when no animation keys the property.
//! Properties the shown animation keys are marked; editing them here
//! changes the rest value, not the keys.
//!
//! The model and the edit helpers are plain data, tested without GTK. The
//! [`Properties`] component shows them and reports changes to the app.

use backstage_core::{
    AnimId, BlendMode, Color, Command, CompId, Entry, NodeId, NodeKind, Project, ProjectSettings, Property,
    Props, Value,
};
use gtk::{gdk, prelude::*};
use relm4::prelude::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

/// The node properties the panel edits, in display order. Color transforms
/// and time offsets come later.
pub const FIELDS: [Property; 13] = [
    Property::X,
    Property::Y,
    Property::Rotation,
    Property::ScaleX,
    Property::ScaleY,
    Property::SkewX,
    Property::SkewY,
    Property::PivotX,
    Property::PivotY,
    Property::Opacity,
    Property::Visible,
    Property::Blend,
    Property::Drawing,
];

pub const BLEND_MODES: [BlendMode; 8] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Add,
    BlendMode::Subtract,
    BlendMode::Overlay,
    BlendMode::Darken,
    BlendMode::Lighten,
];

/// Where `mode` is in [`BLEND_MODES`]. Exhaustive, so a new blend mode
/// can't be forgotten in the dropdown.
pub fn blend_index(mode: BlendMode) -> usize {
    match mode {
        BlendMode::Normal => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Add => 3,
        BlendMode::Subtract => 4,
        BlendMode::Overlay => 5,
        BlendMode::Darken => 6,
        BlendMode::Lighten => 7,
    }
}

/// What the panel shows.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertiesModel {
    Document { name: String, settings: ProjectSettings },
    Node(NodeView),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeView {
    pub id: NodeId,
    pub name: String,
    /// "Shape", "Instance of Ball", …
    pub kind: String,
    pub props: Props,
    /// Which of [`FIELDS`] apply to this node.
    pub fields: Vec<Property>,
    /// Properties the shown animation keys on this node.
    pub animated: BTreeSet<Property>,
    /// The shown animation's name, for the "keyed in" note.
    pub animation: Option<String>,
    /// Number of drawings, for flipbooks.
    pub drawings: u32,
    /// Locked in the editor (its own lock or an ancestor's): only the name
    /// can be edited.
    pub locked: bool,
}

impl PropertiesModel {
    /// The selected node of `comp`, or the document settings if nothing (or
    /// a node that no longer exists) is selected. `name` is the document's
    /// display name; `anim` is the animation the timeline shows.
    pub fn build(
        project: &Project,
        name: &str,
        comp: CompId,
        selection: Option<NodeId>,
        anim: Option<AnimId>,
    ) -> Self {
        let document = || PropertiesModel::Document { name: name.to_owned(), settings: project.settings };
        let Some(comp) = project.compositions.get(&comp) else { return document() };
        let Some((id, node)) = selection.and_then(|id| Some((id, comp.nodes.get(&id)?))) else {
            return document();
        };
        let kind = match &node.kind {
            NodeKind::Group => "Group".to_owned(),
            NodeKind::Shape(_) => "Shape".to_owned(),
            NodeKind::Flipbook(_) => "Flipbook".to_owned(),
            NodeKind::Bitmap(_) => "Bitmap".to_owned(),
            NodeKind::Instance(i) => match project.compositions.get(&i.comp) {
                Some(c) => format!("Instance of {}", c.name),
                None => "Instance".to_owned(),
            },
            NodeKind::Mask => "Mask".to_owned(),
        };
        let anim = anim.and_then(|a| comp.animations.get(&a));
        PropertiesModel::Node(NodeView {
            id,
            name: node.name.clone(),
            kind,
            props: node.rest,
            fields: FIELDS.into_iter().filter(|p| p.applies_to(&node.kind)).collect(),
            animated: anim
                .iter()
                .flat_map(|a| &a.tracks)
                .filter(|t| t.node == id)
                .map(|t| t.property)
                .collect(),
            animation: anim.map(|a| a.name.clone()),
            drawings: match &node.kind {
                NodeKind::Flipbook(drawings) => drawings.len() as u32,
                _ => 0,
            },
            locked: backstage_core::editor::effective(comp).get(&id).is_some_and(|f| f.locked),
        })
    }
}

/// A `SetRest` changing one property of `node`, or `None` if it already
/// has that value (or the node doesn't exist).
pub fn rest_edit(
    project: &Project,
    comp: CompId,
    node: NodeId,
    property: Property,
    value: Value,
) -> Option<Entry> {
    let current = project.compositions.get(&comp)?.nodes.get(&node)?.rest;
    let mut rest = current;
    rest.set(property, value);
    (rest != current).then_some(Entry::Do(Command::SetRest { comp, node, rest }))
}

/// A `RenameNode`, or `None` if the name is empty or unchanged.
pub fn rename(project: &Project, comp: CompId, node: NodeId, name: &str) -> Option<Entry> {
    let current = &project.compositions.get(&comp)?.nodes.get(&node)?.name;
    let name = name.trim();
    (!name.is_empty() && name != current)
        .then(|| Entry::Do(Command::RenameNode { comp, node, name: name.to_owned() }))
}

/// A `SetSettings`, or `None` if nothing changed.
pub fn settings_edit(project: &Project, settings: ProjectSettings) -> Option<Entry> {
    (settings != project.settings).then_some(Entry::Do(Command::SetSettings(settings)))
}

// ---------------------------------------------------------------------------
// GTK

/// How a number field is edited.
struct NumberSpec {
    label: &'static str,
    min: f64,
    max: f64,
    step: f64,
    digits: u32,
}

fn label_of(p: Property) -> &'static str {
    match number_spec(p) {
        Some(spec) => spec.label,
        None => match p {
            Property::Visible => "Visible",
            Property::Blend => "Blend",
            _ => "?",
        },
    }
}

fn number_spec(p: Property) -> Option<NumberSpec> {
    let (label, min, max, step, digits) = match p {
        Property::X => ("X", -1e5, 1e5, 1.0, 2),
        Property::Y => ("Y", -1e5, 1e5, 1.0, 2),
        Property::Rotation => ("Rotation °", -36000.0, 36000.0, 1.0, 1),
        Property::ScaleX => ("Scale X", -1000.0, 1000.0, 0.05, 3),
        Property::ScaleY => ("Scale Y", -1000.0, 1000.0, 0.05, 3),
        Property::SkewX => ("Skew X °", -36000.0, 36000.0, 1.0, 1),
        Property::SkewY => ("Skew Y °", -36000.0, 36000.0, 1.0, 1),
        Property::PivotX => ("Pivot X", -1e5, 1e5, 1.0, 2),
        Property::PivotY => ("Pivot Y", -1e5, 1e5, 1.0, 2),
        Property::Opacity => ("Opacity", 0.0, 1.0, 0.05, 2),
        Property::Drawing => ("Drawing", 0.0, 1e6, 1.0, 0),
        _ => return None,
    };
    Some(NumberSpec { label, min, max, step, digits })
}

enum Field {
    Number(gtk::SpinButton),
    Switch(gtk::Switch),
    Blend(gtk::DropDown),
}

/// One row of the node page.
struct FieldRow {
    label: gtk::Label,
    field: Field,
    keyed: gtk::Label,
}

pub struct Properties {
    model: Rc<RefCell<PropertiesModel>>,
}

#[derive(Debug)]
pub enum PropertiesMsg {
    SetModel(PropertiesModel),
    /// A field's widget changed.
    Edited(Property, Value),
    Renamed(String),
    SettingsEdited(ProjectSettings),
}

#[derive(Debug)]
pub enum PropertiesOutput {
    EditRest { node: NodeId, property: Property, value: Value },
    Rename { node: NodeId, name: String },
    EditSettings(ProjectSettings),
}

pub struct PropertiesWidgets {
    stack: gtk::Stack,
    heading: gtk::Label,
    name: gtk::Entry,
    kind: gtk::Label,
    rows: BTreeMap<Property, FieldRow>,
    doc_name: gtk::Label,
    width: gtk::SpinButton,
    height: gtk::SpinButton,
    background: gtk::ColorDialogButton,
    pixel_art: gtk::Switch,
}

fn key_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(1.0);
    label.add_css_class("dim-label");
    label
}

fn grid() -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.set_row_spacing(6);
    grid.set_column_spacing(10);
    grid.set_margin_start(8);
    grid.set_margin_end(8);
    grid.set_margin_top(4);
    grid.set_margin_bottom(8);
    grid
}

fn spin(min: f64, max: f64, step: f64, digits: u32) -> gtk::SpinButton {
    let spin = gtk::SpinButton::with_range(min, max, step);
    spin.set_digits(digits);
    spin.set_hexpand(true);
    spin
}

/// The node being shown, if any.
fn shown_node(model: &PropertiesModel) -> Option<&NodeView> {
    match model {
        PropertiesModel::Node(view) => Some(view),
        PropertiesModel::Document { .. } => None,
    }
}

impl SimpleComponent for Properties {
    type Init = ();
    type Input = PropertiesMsg;
    type Output = PropertiesOutput;
    type Root = gtk::Box;
    type Widgets = PropertiesWidgets;

    fn init_root() -> Self::Root {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("backstage-panel");
        root
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = Properties {
            model: Rc::new(RefCell::new(PropertiesModel::Document {
                name: String::new(),
                settings: ProjectSettings::default(),
            })),
        };

        let heading = gtk::Label::new(Some("Properties"));
        heading.set_xalign(0.0);
        heading.add_css_class("backstage-panel-title");
        root.append(&heading);

        // Node page.
        let node_grid = grid();
        let name = gtk::Entry::new();
        name.set_hexpand(true);
        let s = sender.clone();
        name.connect_activate(move |e| s.input(PropertiesMsg::Renamed(e.text().to_string())));
        let focus = gtk::EventControllerFocus::new();
        let (s, entry) = (sender.clone(), name.clone());
        focus.connect_leave(move |_| s.input(PropertiesMsg::Renamed(entry.text().to_string())));
        name.add_controller(focus);
        let kind = gtk::Label::new(None);
        kind.set_xalign(0.0);
        node_grid.attach(&key_label("Name"), 0, 0, 1, 1);
        node_grid.attach(&name, 1, 0, 1, 1);
        node_grid.attach(&key_label("Kind"), 0, 1, 1, 1);
        node_grid.attach(&kind, 1, 1, 1, 1);

        let mut rows = BTreeMap::new();
        for (i, p) in FIELDS.into_iter().enumerate() {
            let field = match p {
                Property::Visible => {
                    let switch = gtk::Switch::new();
                    switch.set_halign(gtk::Align::Start);
                    let s = sender.clone();
                    switch.connect_active_notify(move |w| {
                        s.input(PropertiesMsg::Edited(Property::Visible, Value::Bool(w.is_active())))
                    });
                    Field::Switch(switch)
                }
                Property::Blend => {
                    let names: Vec<String> = BLEND_MODES.iter().map(|b| format!("{b:?}")).collect();
                    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                    let drop = gtk::DropDown::from_strings(&refs);
                    let s = sender.clone();
                    drop.connect_selected_notify(move |d| {
                        if let Some(&b) = BLEND_MODES.get(d.selected() as usize) {
                            s.input(PropertiesMsg::Edited(Property::Blend, Value::Blend(b)));
                        }
                    });
                    Field::Blend(drop)
                }
                _ => {
                    let spec = number_spec(p).expect("every other field is a number");
                    let spin = spin(spec.min, spec.max, spec.step, spec.digits);
                    let s = sender.clone();
                    spin.connect_value_changed(move |w| {
                        let value = if p == Property::Drawing {
                            Value::Index(w.value().round().max(0.0) as u32)
                        } else {
                            Value::Number(w.value() as f32)
                        };
                        s.input(PropertiesMsg::Edited(p, value));
                    });
                    Field::Number(spin)
                }
            };
            let label = key_label(label_of(p));
            let keyed = gtk::Label::new(Some("◆"));
            keyed.add_css_class("backstage-keyed");
            let widget: &gtk::Widget = match &field {
                Field::Number(w) => w.upcast_ref(),
                Field::Switch(w) => w.upcast_ref(),
                Field::Blend(w) => w.upcast_ref(),
            };
            let row = i as i32 + 2;
            node_grid.attach(&label, 0, row, 1, 1);
            node_grid.attach(widget, 1, row, 1, 1);
            node_grid.attach(&keyed, 2, row, 1, 1);
            rows.insert(p, FieldRow { label, field, keyed });
        }

        // Document page.
        let doc_grid = grid();
        let doc_name = gtk::Label::new(None);
        doc_name.set_xalign(0.0);
        let width = spin(1.0, 8192.0, 1.0, 0);
        let height = spin(1.0, 8192.0, 1.0, 0);
        let background = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
        background.set_halign(gtk::Align::Start);
        let pixel_art = gtk::Switch::new();
        pixel_art.set_halign(gtk::Align::Start);
        for (row, (key, widget)) in [
            ("Document", doc_name.upcast_ref::<gtk::Widget>()),
            ("Stage width", width.upcast_ref()),
            ("Stage height", height.upcast_ref()),
            ("Background", background.upcast_ref()),
            ("Pixel art", pixel_art.upcast_ref()),
        ]
        .into_iter()
        .enumerate()
        {
            doc_grid.attach(&key_label(key), 0, row as i32, 1, 1);
            doc_grid.attach(widget, 1, row as i32, 1, 1);
        }
        // Each settings widget reports the whole settings with its change.
        let settings_now = {
            let model = model.model.clone();
            move || match &*model.borrow() {
                PropertiesModel::Document { settings, .. } => Some(*settings),
                PropertiesModel::Node(_) => None,
            }
        };
        let (s, now) = (sender.clone(), settings_now.clone());
        width.connect_value_changed(move |w| {
            if let Some(settings) = now() {
                let stage_width = w.value().round() as u32;
                s.input(PropertiesMsg::SettingsEdited(ProjectSettings { stage_width, ..settings }));
            }
        });
        let (s, now) = (sender.clone(), settings_now.clone());
        height.connect_value_changed(move |w| {
            if let Some(settings) = now() {
                let stage_height = w.value().round() as u32;
                s.input(PropertiesMsg::SettingsEdited(ProjectSettings { stage_height, ..settings }));
            }
        });
        let (s, now) = (sender.clone(), settings_now.clone());
        background.connect_rgba_notify(move |b| {
            if let Some(settings) = now() {
                let c = b.rgba();
                let background = Color { r: c.red(), g: c.green(), b: c.blue(), a: c.alpha() };
                s.input(PropertiesMsg::SettingsEdited(ProjectSettings { background, ..settings }));
            }
        });
        let (s, now) = (sender.clone(), settings_now);
        pixel_art.connect_active_notify(move |w| {
            if let Some(settings) = now() {
                s.input(PropertiesMsg::SettingsEdited(ProjectSettings {
                    pixel_art: w.is_active(),
                    ..settings
                }));
            }
        });

        let stack = gtk::Stack::new();
        stack.add_named(&doc_grid, Some("document"));
        stack.add_named(&node_grid, Some("node"));
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&stack));
        root.append(&scroller);

        let widgets = PropertiesWidgets {
            stack,
            heading,
            name,
            kind,
            rows,
            doc_name,
            width,
            height,
            background,
            pixel_art,
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: PropertiesMsg, sender: ComponentSender<Self>) {
        let model = self.model.borrow();
        match msg {
            PropertiesMsg::SetModel(new) => {
                drop(model);
                *self.model.borrow_mut() = new;
            }
            PropertiesMsg::Edited(property, value) => {
                let Some(view) = shown_node(&model) else { return };
                // Spin buttons round to what they show; only a visible change
                // counts, so focusing a field never edits it.
                let changed = match (view.props.get(property), value, number_spec(property)) {
                    (Value::Number(old), Value::Number(new), Some(spec)) => {
                        (old as f64 - new as f64).abs() >= 0.5 * 10f64.powi(-(spec.digits as i32))
                    }
                    (old, new, _) => old != new,
                };
                if changed {
                    let _ = sender.output(PropertiesOutput::EditRest { node: view.id, property, value });
                }
            }
            PropertiesMsg::Renamed(name) => {
                let Some(view) = shown_node(&model) else { return };
                if name.trim() != view.name && !name.trim().is_empty() {
                    let _ = sender.output(PropertiesOutput::Rename { node: view.id, name });
                }
            }
            PropertiesMsg::SettingsEdited(settings) => {
                if let PropertiesModel::Document { settings: current, .. } = &*model
                    && settings != *current
                {
                    let _ = sender.output(PropertiesOutput::EditSettings(settings));
                }
            }
        }
    }

    fn update_view(&self, w: &mut Self::Widgets, _sender: ComponentSender<Self>) {
        // Setting a widget to the model's value fires its change signal;
        // `update` sees no change and sends nothing.
        let model = self.model.borrow().clone();
        match &model {
            PropertiesModel::Document { name, settings } => {
                w.stack.set_visible_child_name("document");
                w.heading.set_label("Properties · Document");
                w.doc_name.set_label(name);
                w.width.set_value(settings.stage_width as f64);
                w.height.set_value(settings.stage_height as f64);
                let c = settings.background;
                if w.background.rgba().to_str() != gdk::RGBA::new(c.r, c.g, c.b, c.a).to_str() {
                    w.background.set_rgba(&gdk::RGBA::new(c.r, c.g, c.b, c.a));
                }
                w.pixel_art.set_active(settings.pixel_art);
            }
            PropertiesModel::Node(view) => {
                w.stack.set_visible_child_name("node");
                w.heading.set_label(&format!("Properties · {}", view.name));
                if w.name.text() != view.name {
                    w.name.set_text(&view.name);
                }
                w.kind.set_label(&if view.locked {
                    format!("{} · Locked", view.kind)
                } else {
                    view.kind.clone()
                });
                for (p, row) in &w.rows {
                    let shown = view.fields.contains(p);
                    row.label.set_visible(shown);
                    let widget: &gtk::Widget = match &row.field {
                        Field::Number(w) => w.upcast_ref(),
                        Field::Switch(w) => w.upcast_ref(),
                        Field::Blend(w) => w.upcast_ref(),
                    };
                    widget.set_sensitive(!view.locked);
                    row.keyed.set_visible(shown && view.animated.contains(p));
                    if let Some(anim) = &view.animation {
                        row.keyed.set_tooltip_text(Some(&format!(
                            "Keyed in “{anim}”: this edits the rest value, which shows when no animation keys it."
                        )));
                    }
                    match (&row.field, view.props.get(*p)) {
                        (Field::Number(spin), value) => {
                            spin.set_visible(shown);
                            if *p == Property::Drawing {
                                spin.set_range(0.0, view.drawings.saturating_sub(1) as f64);
                            }
                            let v = match value {
                                Value::Number(v) => v as f64,
                                Value::Index(i) => i as f64,
                                _ => 0.0,
                            };
                            spin.set_value(v);
                        }
                        (Field::Switch(switch), Value::Bool(on)) => {
                            switch.set_visible(shown);
                            switch.set_active(on);
                        }
                        (Field::Blend(drop), Value::Blend(b)) => {
                            drop.set_visible(shown);
                            drop.set_selected(blend_index(b) as u32);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backstage_core::sample::{self, ids::*};

    fn node(model: PropertiesModel) -> NodeView {
        match model {
            PropertiesModel::Node(view) => view,
            other => panic!("expected a node, got {other:?}"),
        }
    }

    #[test]
    fn nothing_selected_shows_the_document() {
        let project = sample::bounce();
        assert_eq!(
            PropertiesModel::build(&project, "bounce", STAGE, None, Some(STAGE_MAIN)),
            PropertiesModel::Document { name: "bounce".into(), settings: project.settings }
        );
        let gone = backstage_core::NodeId::from_raw(1);
        assert!(matches!(
            PropertiesModel::build(&project, "bounce", STAGE, Some(gone), None),
            PropertiesModel::Document { .. }
        ));
    }

    #[test]
    fn a_node_shows_its_rest_values_and_what_the_animation_keys() {
        let project = sample::bounce();
        let ground = node(PropertiesModel::build(&project, "", STAGE, Some(GROUND), Some(STAGE_MAIN)));
        assert_eq!((ground.name.as_str(), ground.kind.as_str()), ("ground", "Shape"));
        assert_eq!(ground.props, project.compositions[&STAGE].nodes[&GROUND].rest);
        assert!(!ground.fields.contains(&Property::Drawing), "not a flipbook");
        assert_eq!(ground.fields.len(), FIELDS.len() - 1);
        assert!(ground.animated.is_empty());

        let synced = node(PropertiesModel::build(&project, "", STAGE, Some(SYNCED_BALL), Some(STAGE_MAIN)));
        assert_eq!(synced.animated, BTreeSet::from([Property::X]));
        assert_eq!(synced.animation.as_deref(), Some("main"));
        assert_eq!(synced.kind, "Instance of Ball");
        let unanimated = node(PropertiesModel::build(&project, "", STAGE, Some(SYNCED_BALL), None));
        assert!(unanimated.animated.is_empty());

        let eye = node(PropertiesModel::build(&project, "", BLINKER, Some(BLINKER_EYE), None));
        assert_eq!(eye.kind, "Flipbook");
        assert!(eye.fields.contains(&Property::Drawing));
        assert!(eye.drawings > 1);
    }

    #[test]
    fn a_locked_node_or_a_child_of_one_is_locked() {
        let mut project = sample::bounce();
        let open = node(PropertiesModel::build(&project, "", STAGE, Some(GROUND), None));
        assert!(!open.locked);
        let stage = project.compositions.get_mut(&STAGE).unwrap();
        stage.nodes.get_mut(&STAGE_ROOT).unwrap().editor.locked = true;
        let inherited = node(PropertiesModel::build(&project, "", STAGE, Some(GROUND), None));
        assert!(inherited.locked, "locked through the root group");
    }

    #[test]
    fn rest_edits_change_one_property_and_undo_exactly() {
        let project = sample::bounce();
        let Some(Entry::Do(cmd)) =
            rest_edit(&project, STAGE, GROUND, Property::Rotation, Value::Number(30.0))
        else {
            panic!("expected an edit")
        };
        let mut edited = project.clone();
        let inverse = cmd.apply(&mut edited).unwrap();
        let (before, after) = (
            project.compositions[&STAGE].nodes[&GROUND].rest,
            edited.compositions[&STAGE].nodes[&GROUND].rest,
        );
        assert_eq!(after.transform.rotation, 30.0);
        for p in Property::ALL.into_iter().filter(|&p| p != Property::Rotation) {
            assert_eq!(after.get(p), before.get(p), "{p:?} changed");
        }
        inverse.apply(&mut edited).unwrap();
        assert_eq!(edited, project);

        let current = before.get(Property::X);
        assert_eq!(rest_edit(&project, STAGE, GROUND, Property::X, current), None, "unchanged");
        let gone = backstage_core::NodeId::from_raw(1);
        assert_eq!(rest_edit(&project, STAGE, gone, Property::X, Value::Number(1.0)), None);
    }

    #[test]
    fn renames_skip_empty_and_unchanged_names() {
        let project = sample::bounce();
        assert_eq!(rename(&project, STAGE, GROUND, "ground"), None);
        assert_eq!(rename(&project, STAGE, GROUND, "   "), None);
        let Some(Entry::Do(cmd)) = rename(&project, STAGE, GROUND, " floor ") else { panic!() };
        let mut edited = project.clone();
        cmd.apply(&mut edited).unwrap();
        assert_eq!(edited.compositions[&STAGE].nodes[&GROUND].name, "floor");
    }

    #[test]
    fn settings_edits_apply_and_skip_no_ops() {
        let project = sample::bounce();
        assert_eq!(settings_edit(&project, project.settings), None);
        let settings =
            ProjectSettings { background: Color::rgb8(10, 20, 30), stage_width: 640, ..project.settings };
        let Some(Entry::Do(cmd)) = settings_edit(&project, settings) else { panic!() };
        let mut edited = project.clone();
        cmd.apply(&mut edited).unwrap();
        assert_eq!(edited.settings, settings);
    }

    #[test]
    fn every_blend_mode_is_offered_in_order() {
        for (i, mode) in BLEND_MODES.into_iter().enumerate() {
            assert_eq!(blend_index(mode), i, "{mode:?}");
        }
    }
}
