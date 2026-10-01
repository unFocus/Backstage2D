//! Placeholder panels. They show the intended layout; none of them are wired
//! to a document yet. (The timeline and properties are real: see `timeline.rs`
//! and `properties.rs`.)

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
