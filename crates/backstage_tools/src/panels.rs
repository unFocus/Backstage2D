//! Placeholder panels. They show the intended layout; none of them are wired
//! to a document yet.

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

const LAYERS: [&str; 3] = ["Actions", "Ball", "Background"];
const HEADER_H: f64 = 22.0;
const ROW_H: f64 = 22.0;
const NAME_W: f64 = 140.0;
const CELL_W: f64 = 10.0;
const PLAYHEAD_FRAME: u32 = 12;

/// Layers × frames grid with a playhead. Seed of the real timeline widget.
pub fn timeline() -> gtk::Box {
    let area = gtk::DrawingArea::new();
    area.set_vexpand(true);
    area.set_hexpand(true);
    area.set_content_height((HEADER_H + ROW_H * LAYERS.len() as f64) as i32 + 8);
    area.set_draw_func(|_, cr, width, height| {
        let (width, height) = (width as f64, height as f64);
        let _ = draw_timeline(cr, width, height);
    });
    panel("Timeline", &area)
}

fn draw_timeline(cr: &gtk::cairo::Context, width: f64, height: f64) -> Result<(), gtk::cairo::Error> {
    cr.set_source_rgb(0.16, 0.16, 0.18);
    cr.paint()?;

    let frames = ((width - NAME_W) / CELL_W).max(0.0) as u32;
    cr.set_font_size(11.0);

    // Frame cells, shading every 5th frame like Flash.
    for f in 0..frames {
        let x = NAME_W + f as f64 * CELL_W;
        if (f + 1) % 5 == 0 {
            cr.set_source_rgb(0.22, 0.22, 0.25);
            cr.rectangle(x, HEADER_H, CELL_W, height - HEADER_H);
            cr.fill()?;
        }
        if f == 0 || (f + 1) % 5 == 0 {
            cr.set_source_rgb(0.65, 0.65, 0.7);
            cr.move_to(x + 1.0, HEADER_H - 7.0);
            cr.show_text(&(f + 1).to_string())?;
        }
    }
    cr.set_source_rgb(0.28, 0.28, 0.31);
    cr.set_line_width(1.0);
    for f in 0..=frames {
        let x = (NAME_W + f as f64 * CELL_W).floor() + 0.5;
        cr.move_to(x, HEADER_H);
        cr.line_to(x, height);
    }
    for row in 0..=LAYERS.len() {
        let y = (HEADER_H + row as f64 * ROW_H).floor() + 0.5;
        cr.move_to(0.0, y);
        cr.line_to(width, y);
    }
    cr.stroke()?;

    // Layer names, a keyframe span per layer.
    for (row, name) in LAYERS.iter().enumerate() {
        let y = HEADER_H + row as f64 * ROW_H;
        cr.set_source_rgb(0.85, 0.85, 0.88);
        cr.move_to(8.0, y + ROW_H - 7.0);
        cr.show_text(name)?;

        let span = [1, 30, 60][row].min(frames);
        cr.set_source_rgba(0.35, 0.55, 0.85, 0.35);
        cr.rectangle(NAME_W, y + 1.0, span as f64 * CELL_W, ROW_H - 1.0);
        cr.fill()?;
        cr.set_source_rgb(0.9, 0.9, 0.95);
        cr.arc(NAME_W + CELL_W / 2.0, y + ROW_H - 6.0, 2.5, 0.0, std::f64::consts::TAU);
        cr.fill()?;
    }

    // Playhead.
    let x = NAME_W + (PLAYHEAD_FRAME as f64 - 0.5) * CELL_W;
    cr.set_source_rgb(0.95, 0.25, 0.3);
    cr.rectangle(x - 4.0, 2.0, 8.0, HEADER_H - 6.0);
    cr.fill()?;
    cr.set_line_width(1.0);
    cr.move_to(x.floor() + 0.5, HEADER_H - 4.0);
    cr.line_to(x.floor() + 0.5, height);
    cr.stroke()
}
