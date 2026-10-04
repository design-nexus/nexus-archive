//! A container that keeps its child at most a given width and centred, without asking
//! for more than the child's own minimum (side margins would hold a narrow window open).

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    pub struct Clamp {
        pub max: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Clamp {
        const NAME: &'static str = "NarcClamp";
        type Type = super::Clamp;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Clamp {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Clamp {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(c) = self.obj().first_child() else { return (0, 0, -1, -1) };
            let max = self.max.get();
            if orientation == gtk::Orientation::Horizontal {
                let (min, nat, _, _) = c.measure(orientation, for_size);
                (min, nat.min(max).max(min), -1, -1)
            } else {
                let width = if for_size < 0 { for_size } else { for_size.min(max).max(c.measure(gtk::Orientation::Horizontal, -1).0) };
                let (min, nat, _, _) = c.measure(orientation, width);
                (min, nat, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let Some(c) = self.obj().first_child() else { return };
            let cw = width.min(self.max.get()).max(c.measure(gtk::Orientation::Horizontal, -1).0);
            let x = ((width - cw) / 2).max(0);
            c.size_allocate(&gtk::Allocation::new(x, 0, cw, height), -1);
        }
    }
}

glib::wrapper! {
    pub struct Clamp(ObjectSubclass<imp::Clamp>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Clamp {
    pub fn new(child: &impl IsA<gtk::Widget>, max: i32) -> Self {
        let c: Self = glib::Object::new();
        c.imp().max.set(max);
        child.set_parent(&c);
        c
    }
}
