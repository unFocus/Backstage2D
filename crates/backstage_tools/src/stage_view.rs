//! The stage section: shows the latest frame streamed from the stage process
//! and reports its size in physical pixels so the stage can render to match.

use gtk::{gdk, glib, graphene, prelude::*, subclass::prelude::*};
use relm4::gtk;

type ResizedCallback = Box<dyn Fn(u32, u32, f64)>;

glib::wrapper! {
    pub struct StageView(ObjectSubclass<imp::StageView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for StageView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl StageView {
    pub fn set_frame(&self, texture: gdk::Texture) {
        self.imp().texture.replace(Some(texture));
        self.queue_draw();
    }

    pub fn clear_frame(&self) {
        self.imp().texture.replace(None);
        self.queue_draw();
    }

    /// Called with (width, height, scale) whenever the physical size changes.
    pub fn connect_stage_resized(&self, f: impl Fn(u32, u32, f64) + 'static) {
        self.imp().on_resized.replace(Some(Box::new(f)));
        self.report_size();
    }

    /// The last reported (width, height, scale), if any.
    pub fn stage_size(&self) -> Option<(u32, u32, f64)> {
        self.imp().last_size.get()
    }

    fn report_size(&self) {
        let scale = self
            .native()
            .and_then(|n| n.surface())
            .map(|s| s.scale())
            .unwrap_or_else(|| self.scale_factor() as f64);
        let width = (self.width() as f64 * scale).round() as u32;
        let height = (self.height() as f64 * scale).round() as u32;
        let size = Some((width, height, scale));
        let imp = self.imp();
        if imp.last_size.get() == size {
            return;
        }
        imp.last_size.set(size);
        if let Some(f) = imp.on_resized.borrow().as_ref() {
            f(width, height, scale);
        }
    }
}

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct StageView {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub on_resized: RefCell<Option<ResizedCallback>>,
        pub last_size: Cell<Option<(u32, u32, f64)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StageView {
        const NAME: &'static str = "BackstageStageView";
        type Type = super::StageView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for StageView {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_hexpand(true);
            obj.set_vexpand(true);
            obj.set_focusable(true);
        }
    }

    impl WidgetImpl for StageView {
        fn realize(&self) {
            self.parent_realize();
            // Fractional scale can change without a size change (moving the
            // window to another monitor).
            if let Some(surface) = self.obj().native().and_then(|n| n.surface()) {
                let obj = self.obj().downgrade();
                surface.connect_scale_notify(move |_| {
                    if let Some(obj) = obj.upgrade() {
                        obj.report_size();
                    }
                });
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.obj().report_size();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let bounds = graphene::Rect::new(0.0, 0.0, obj.width() as f32, obj.height() as f32);
            match self.texture.borrow().as_ref() {
                Some(texture) => snapshot.append_texture(texture, &bounds),
                None => snapshot.append_color(&gdk::RGBA::new(0.23, 0.23, 0.25, 1.0), &bounds),
            }
        }
    }
}
