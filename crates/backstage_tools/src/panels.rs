//! Placeholder panels. They show the intended layout; none of them are wired
//! to a document yet. (The timeline is real: see `timeline.rs`.)

use gtk::prelude::*;
use relm4::gtk;

fn panel(title: &str, body: &impl IsA<gtk::Widget>) -> gtk::Box {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    panel.add_css_class("backstage-panel");
    let heading = gtk::Label::new(Some(title));
    heading.set_xalign(0.0);
    heading.add_css_class("backstage-panel-title");
    panel.append(&heading);
    panel.append(body);
    panel
}

pub fn library() -> gtk::Box {
    let list = gtk::ListBox::new();
    list.set_vexpand(true);
    for (name, kind) in
        [("Ball", "MovieClip"), ("Background", "Graphic"), ("PlayButton", "Button"), ("logo.png", "Bitmap")]
    {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_margin_start(8);
        row.set_margin_end(8);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        let name = gtk::Label::new(Some(name));
        name.set_hexpand(true);
        name.set_xalign(0.0);
        let kind = gtk::Label::new(Some(kind));
        kind.add_css_class("dim-label");
        row.append(&name);
        row.append(&kind);
        list.append(&row);
    }
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&list));
    scroller.set_vexpand(true);
    panel("Library", &scroller)
}

pub fn properties() -> gtk::Box {
    let grid = gtk::Grid::new();
    grid.set_row_spacing(6);
    grid.set_column_spacing(12);
    grid.set_margin_start(8);
    grid.set_margin_end(8);
    grid.set_margin_top(8);
    let (w, h) = (550, 400); // backstage_render::STAGE_SIZE; tools doesn't link the renderer
    for (row, (key, value)) in [
        ("Document", "Untitled".to_string()),
        ("Stage", format!("{w} × {h}")),
        ("Frame rate", "24 fps".into()),
        ("Background", "#FFFFFF".into()),
    ]
    .into_iter()
    .enumerate()
    {
        let key = gtk::Label::new(Some(key));
        key.set_xalign(1.0);
        key.add_css_class("dim-label");
        let value = gtk::Label::new(Some(&value));
        value.set_xalign(0.0);
        grid.attach(&key, 0, row as i32, 1, 1);
        grid.attach(&value, 1, row as i32, 1, 1);
    }
    panel("Properties", &grid)
}
