//! The settings dialog: the window's theme and where files are extracted.

use crate::{paths, prefs, theme, widgets, window};
use gtk::glib;
use gtk::prelude::*;

pub const SHORTCUTS: &[(&[&str], &str)] = &[
    (&["Ctrl", "O"], "Open an archive"),
    (&["Ctrl", "N"], "New archive"),
    (&["Ctrl", "E"], "Extract the selection, or everything"),
    (&["Ctrl", "F"], "Search the archive"),
    (&["Ctrl", "A"], "Select all"),
    (&["Enter"], "Open the folder or file"),
    (&["Backspace"], "Up one folder"),
    (&["Delete"], "Delete from the archive"),
    (&["Menu"], "More actions for the selection (or right-click)"),
    (&["Esc"], "Clear the search, unselect, or go back"),
    (&["Ctrl", ","], "Settings"),
    (&["?"], "This list"),
    (&["Ctrl", "Q"], "Close"),
];

/// The keyboard shortcuts on their own, for `?` and F1.
pub fn show_help() {
    let (dialog, card) = widgets::dialog("Keyboard shortcuts", 460);
    let list = widgets::vbox(0);
    list.add_css_class("help-list");
    for (keys, what) in SHORTCUTS {
        let row = widgets::hbox(12);
        row.add_css_class("help-row");
        let l = widgets::label(what, "");
        l.set_hexpand(true);
        l.set_wrap(true);
        row.append(&l);
        row.append(&widgets::key_caps(keys));
        list.append(&row);
    }
    card.append(&list);
    let tip = widgets::label("Drop files on the window to compress them, or onto an open archive to add them.", "dim");
    tip.set_wrap(true);
    card.append(&tip);
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    close.add_css_class("suggested-action");
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    card.append(&close);
    dialog.present();
    close.grab_focus();
}

pub fn show() {
    let (dialog, card) = widgets::dialog("Settings", 560);
    dialog.set_default_height(640);
    let p = prefs::get();

    let body = widgets::vbox(0);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&body)
        .vexpand(true)
        .build();

    let group = |title: &str| {
        let g = widgets::vbox(6);
        let t = widgets::label(&title.to_uppercase(), "group-title");
        body.append(&t);
        body.append(&g);
        g
    };

    // ----- Extracting -----
    let g = group("Extracting");
    let (r, _) = widgets::switch_row("Extract into a new folder", "Files go into a folder named after the archive, so nothing gets mixed in.", p.extract_into_folder, |on| {
        prefs::update(|p| p.extract_into_folder = on);
    });
    g.append(&r);
    let dest_label = if p.extract_dir.is_empty() { "Next to the archive".to_string() } else { paths::pretty(std::path::Path::new(&p.extract_dir)) };
    let dest = gtk::Button::with_label(&dest_label);
    dest.add_css_class("path-button");
    dest.connect_clicked(|b| {
        let b = b.clone();
        window::choose_folder("Extract into", move |dir| {
            b.set_label(&paths::pretty(&dir));
            prefs::update(|p| p.extract_dir = dir.to_string_lossy().to_string());
        });
    });
    g.append(&widgets::row("Extract to", "Where extracted files go.", Some(dest.upcast_ref())));
    let reset = gtk::Button::with_label("Use the archive's folder");
    {
        let dest = dest.clone();
        reset.connect_clicked(move |_| {
            dest.set_label("Next to the archive");
            prefs::update(|p| p.extract_dir.clear());
        });
    }
    g.append(&reset);
    reset.set_halign(gtk::Align::End);
    let (r, _) = widgets::choice_row(
        "If a file already exists",
        "What happens to files with the same name.",
        widgets::opts(&[("ask", "Ask me"), ("replace", "Replace"), ("skip", "Keep existing"), ("rename", "Keep both")]),
        &p.overwrite,
        |id| prefs::update(|p| p.overwrite = id),
    );
    g.append(&r);

    // ----- Archive window -----
    let g = group("Archive window");
    let themes = theme::all();
    let options: Vec<(String, String)> = themes.iter().map(|t| (t.id.clone(), t.name.clone())).collect();
    let (theme_row, theme_dd) = widgets::choice_row(
        "Theme",
        "Dracula, Catppuccin, Tokyo Night and more. Add your own in <tt>~/.config/nexus-archive/themes</tt>.",
        options,
        &p.theme,
        |id| {
            prefs::update(|p| {
                p.theme = id;
                p.mode = prefs::ThemeMode::Theme;
            });
            theme::apply();
        },
    );
    theme_dd.set_sensitive(p.mode == prefs::ThemeMode::Theme || !theme::omarchy_available());
    if theme::omarchy_available() {
        let dd = theme_dd.clone();
        let (r, _) = widgets::switch_row(
            "Follow Omarchy theme",
            "Match the desktop's colours and update live whenever the Omarchy theme changes.",
            p.mode == prefs::ThemeMode::Omarchy,
            move |on| {
                prefs::update(|p| p.mode = if on { prefs::ThemeMode::Omarchy } else { prefs::ThemeMode::Theme });
                dd.set_sensitive(!on);
                theme::apply();
            },
        );
        g.append(&r);
    }
    g.append(&theme_row);

    let swatches = widgets::hbox(4);
    let refresh_swatches = {
        let swatches = swatches.clone();
        move || {
            while let Some(c) = swatches.first_child() {
                swatches.remove(&c);
            }
            let pal = theme::current_palette();
            for c in [&pal.bg, &pal.surface, &pal.muted, &pal.text, &pal.accent, &pal.danger] {
                let s = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                s.add_css_class("swatch");
                let provider = gtk::CssProvider::new();
                provider.load_from_string(&format!("box {{ background: {c}; }}"));
                #[allow(deprecated)]
                s.style_context().add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                swatches.append(&s);
            }
        }
    };
    refresh_swatches();
    let last = std::cell::RefCell::new(theme::current_palette());
    let weak = swatches.downgrade();
    glib::timeout_add_seconds_local(1, move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        let now = theme::current_palette();
        if *last.borrow() != now {
            *last.borrow_mut() = now;
            refresh_swatches();
        }
        glib::ControlFlow::Continue
    });
    g.append(&widgets::row("Current colours", "", Some(swatches.upcast_ref())));
    let (r, _) = widgets::switch_row("Glow", "Soft accent glow around focused and selected elements.", p.glow, |on| {
        prefs::update(|p| p.glow = on);
        theme::apply();
    });
    g.append(&r);
    let (r, _) = widgets::switch_row("Reduce motion", "Turn off transitions and animations in this window.", p.reduce_motion, |on| {
        prefs::update(|p| p.reduce_motion = on);
        theme::apply();
    });
    g.append(&r);

    // ----- Keyboard -----
    let g = group("Keyboard");
    for (keys, what) in SHORTCUTS {
        g.append(&widgets::row(what, "", Some(widgets::key_caps(keys).upcast_ref())));
    }

    card.append(&scroll);
    let close = gtk::Button::with_label("Done");
    close.set_halign(gtk::Align::End);
    close.add_css_class("suggested-action");
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    card.append(&close);
    dialog.present();
}
