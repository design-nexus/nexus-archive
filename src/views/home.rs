//! The start view: a drop zone and the recent archives.

use crate::sevenzip;
use crate::{fmt, paths, recent, widgets, window};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct Home {
    pub root: gtk::ScrolledWindow,
    recents: gtk::Box,
    recents_group: gtk::Box,
    banner: gtk::Box,
}

/// Files dropped onto a drop target.
pub fn dropped(value: &glib::Value) -> Vec<PathBuf> {
    value
        .get::<gdk::FileList>()
        .map(|l| l.files().iter().filter_map(|f| f.path()).collect())
        .unwrap_or_default()
}

/// What to do with dropped files: a lone archive opens, anything else is compressed.
pub fn handle_drop(files: Vec<PathBuf>) {
    if files.is_empty() {
        return;
    }
    if files.len() == 1 && files[0].is_file() && sevenzip::looks_like_archive(&files[0]) {
        window::open_archive(files[0].clone());
    } else {
        window::start_create(files);
    }
}

impl Home {
    pub fn new() -> Rc<Self> {
        let body = widgets::vbox(0);
        body.add_css_class("settings-page");

        let banner = widgets::banner(
            "<b>7-Zip isn't installed.</b> Install the <tt>7zip</tt> package (<tt>sudo pacman -S 7zip</tt>), then reopen this window.",
            true,
        );
        banner.set_visible(false);
        banner.set_margin_top(14);
        body.append(&banner);

        // ----- Drop zone -----
        let zone = widgets::vbox(10);
        zone.add_css_class("dropzone");
        zone.set_margin_top(18);
        zone.set_halign(gtk::Align::Fill);
        let icon = gtk::Image::from_icon_name("package-x-generic-symbolic");
        icon.set_pixel_size(44);
        icon.add_css_class("dropzone-icon");
        zone.append(&icon);
        let title = widgets::label("Drop files here", "dropzone-title");
        title.set_halign(gtk::Align::Center);
        zone.append(&title);
        let sub = widgets::label("Drop an archive to open it, or any other files to compress them.", "dim");
        sub.set_halign(gtk::Align::Center);
        sub.set_wrap(true);
        sub.set_justify(gtk::Justification::Center);
        zone.append(&sub);
        let buttons = widgets::hbox(10);
        buttons.set_halign(gtk::Align::Center);
        buttons.set_margin_top(10);
        let open = widgets::labeled_button("document-open-symbolic", "Open archive");
        open.add_css_class("suggested-action");
        open.connect_clicked(|_| choose_archive());
        let new = widgets::labeled_button("list-add-symbolic", "New archive");
        new.connect_clicked(|_| window::start_create(Vec::new()));
        buttons.append(&open);
        buttons.append(&new);
        zone.append(&buttons);
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let z = zone.clone();
        target.connect_enter(move |_, _, _| {
            z.add_css_class("drop-hover");
            gdk::DragAction::COPY
        });
        let z = zone.clone();
        target.connect_leave(move |_| z.remove_css_class("drop-hover"));
        let z = zone.clone();
        target.connect_drop(move |_, value, _, _| {
            z.remove_css_class("drop-hover");
            handle_drop(dropped(value));
            true
        });
        zone.add_controller(target);
        body.append(&zone);

        // ----- Recent -----
        let recents_group = widgets::vbox(0);
        let head = widgets::hbox(8);
        let t = widgets::label("RECENT", "group-title");
        t.set_hexpand(true);
        head.append(&t);
        let clear = gtk::Button::with_label("Clear list");
        clear.add_css_class("flat");
        clear.add_css_class("small-button");
        clear.set_valign(gtk::Align::End);
        clear.set_margin_bottom(4);
        head.append(&clear);
        recents_group.append(&head);
        let recents = widgets::vbox(6);
        recents_group.append(&recents);
        body.append(&recents_group);

        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&widgets::clamp(&body, 920))
            .vexpand(true)
            .build();
        let home = Rc::new(Home { root, recents, recents_group, banner });
        let h = home.clone();
        clear.connect_clicked(move |_| {
            for p in recent::list() {
                recent::forget(&p);
            }
            h.refresh();
            window::toast("Cleared the recent list. The archives themselves are untouched.");
        });
        home.refresh();
        home
    }

    pub fn show_missing_tool(&self) {
        self.banner.set_visible(true);
    }

    pub fn choose_archive(&self) {
        choose_archive();
    }

    pub fn refresh(&self) {
        while let Some(c) = self.recents.first_child() {
            self.recents.remove(&c);
        }
        let list = recent::list();
        self.recents_group.set_visible(!list.is_empty());
        for path in list {
            self.recents.append(&recent_row(&path, self));
        }
        window::apply_narrow();
    }
}

pub fn choose_archive() {
    window::choose_files("Open archive", true, false, |files| window::open_archive(files[0].clone()));
}

fn recent_row(path: &Path, home: &Home) -> gtk::Box {
    let card = widgets::hbox(0);
    card.add_css_class("recent-row");
    let open = gtk::Button::new();
    open.add_css_class("flat");
    open.set_hexpand(true);
    let c = widgets::hbox(12);
    let icon = gtk::Image::from_icon_name("package-x-generic-symbolic");
    icon.add_css_class("accent-text");
    c.append(&icon);
    let text = widgets::vbox(1);
    text.set_hexpand(true);
    let name = widgets::label(&path.file_name().unwrap_or_default().to_string_lossy(), "settings-option-title");
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let dir = path.parent().map(paths::pretty).unwrap_or_default();
    let sub = widgets::label(&dir, "mono");
    sub.add_css_class("settings-option-description");
    sub.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    text.append(&name);
    text.append(&sub);
    c.append(&text);
    if let Ok(meta) = std::fs::metadata(path) {
        let kind = path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
        let when = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| glib::DateTime::from_unix_local(d.as_secs() as i64).ok())
            .and_then(|d| d.format("%-d %b %Y").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let facts = widgets::label(&format!("{kind} · {} · {when}", fmt::size(meta.len())), "mono");
        facts.add_css_class("dim");
        facts.add_css_class("recent-facts");
        facts.add_css_class("col-hide-narrow");
        facts.set_visible(window::fits("col-hide-narrow"));
        c.append(&facts);
    }
    open.set_child(Some(&c));
    let p = path.to_path_buf();
    open.connect_clicked(move |_| window::open_archive(p.clone()));
    card.append(&open);
    let remove = widgets::icon_button("window-close-symbolic", "Remove from recent");
    let p = path.to_path_buf();
    let weak = card.downgrade();
    let _ = home;
    remove.connect_clicked(move |_| {
        recent::forget(&p);
        if let Some(card) = weak.upgrade()
            && let Some(parent) = card.parent().and_downcast::<gtk::Box>()
        {
            parent.remove(&card);
        }
    });
    card.append(&remove);
    card
}
