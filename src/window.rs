//! The main window: a stack of three views (home, browse, create) with a progress
//! card and toasts floating over it.

use crate::sevenzip::job::{self, Cancel, Event, Op, Outcome};
use crate::views::{browse::Browse, create::Create, home::Home};
use crate::{paths, prefs, sevenzip, theme, widgets};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

/// Under this width tables shed columns and pages use the narrow padding.
const NARROW_WIDTH: i32 = 860;

/// Classes that hide below a window width: toolbar labels go one at a time, least
/// important first, so a half-screen tile still shows "Extract" and "Add".
const SHED: [(&str, i32); 6] = [
    ("label-low", 1000),
    ("col-modified", NARROW_WIDTH),
    ("col-packed", NARROW_WIDTH),
    ("col-hide-narrow", NARROW_WIDTH),
    ("label-mid", 760),
    ("label-high", 640),
];

struct Progress {
    root: gtk::Box,
    open_when_done: gtk::CheckButton,
    title: gtk::Label,
    current: gtk::Label,
    readout: gtk::Label,
    bar: gtk::ProgressBar,
}

struct Ui {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    overlay: gtk::Overlay,
    progress: Progress,
    home: Rc<Home>,
    browse: Rc<Browse>,
    create: Rc<Create>,
    cancel: RefCell<Option<Cancel>>,
}

thread_local! {
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
    static WIDTH: Cell<i32> = const { Cell::new(0) };
    static TOAST: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
}

fn ui() -> Option<Rc<Ui>> {
    UI.with(|u| u.borrow().clone())
}

/// What the command line asked for.
#[derive(Default)]
pub struct Request {
    pub files: Vec<PathBuf>,
    /// Compress the files instead of opening them.
    pub create: bool,
}

pub fn present(app: &gtk::Application, req: Request) {
    if ui().is_none() {
        theme::install();
        build(app);
        if prefs::take_broken() {
            toast("Your settings file couldn't be read, so defaults are in use. The old file is kept as settings.toml.bak.");
        }
        if sevenzip::binary().is_none() {
            show_missing_banner();
        }
    }
    let Some(ui) = ui() else { return };
    ui.window.present();
    if req.files.is_empty() {
        return;
    }
    if req.create {
        ui.create.start(req.files);
    } else {
        ui.browse.open(req.files[0].clone());
    }
}

fn show_missing_banner() {
    if let Some(ui) = ui() {
        ui.home.show_missing_tool();
    }
}

pub fn window() -> Option<gtk::ApplicationWindow> {
    ui().map(|u| u.window.clone())
}

pub fn show(view: &str) {
    if let Some(ui) = ui() {
        ui.stack.set_visible_child_name(view);
        if view == "home" {
            ui.home.refresh();
        }
    }
}

pub fn current() -> String {
    ui().and_then(|u| u.stack.visible_child_name()).map(|n| n.to_string()).unwrap_or_default()
}

fn shed_in(w: &gtk::Widget, width: i32) {
    for (class, below) in SHED {
        if w.has_css_class(class) {
            w.set_visible(width >= below);
        }
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        shed_in(&c, width);
        child = c.next_sibling();
    }
}

/// Hide the columns and labels that don't fit the window. Views call this after they
/// build rows, so new rows match the current width.
pub fn apply_narrow() {
    if let Some(ui) = ui() {
        shed_in(ui.stack.upcast_ref(), WIDTH.with(Cell::get));
    }
}

/// Whether a widget with this class should show at the current width (for rows made
/// while the window is already narrow).
pub fn fits(class: &str) -> bool {
    let width = WIDTH.with(Cell::get);
    SHED.iter().find(|(c, _)| *c == class).is_none_or(|(_, below)| width == 0 || width >= *below)
}

fn build(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::builder().application(app).title("Nexus Archive").default_width(1040).default_height(720).build();
    window.add_css_class("archive-window");
    // No client-side titlebar: Hyprland manages the window.
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    window.set_icon_name(Some(crate::APP_ID));
    window.set_size_request(520, 420);

    let home = Home::new();
    let browse = Browse::new();
    let create = Create::new();

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(160);
    stack.add_named(&home.root, Some("home"));
    stack.add_named(&browse.root, Some("browse"));
    stack.add_named(&create.root, Some("create"));
    stack.set_visible_child_name("home");

    let progress = build_progress();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&stack));
    overlay.add_overlay(&progress.root);
    window.set_child(Some(&overlay));

    // The window has no resize signal worth trusting, so a tick callback watches the width.
    window.add_tick_callback(|w, _| {
        let width = w.width();
        let tier = |x: i32| SHED.iter().filter(|(_, below)| x < *below).count();
        let old = WIDTH.with(Cell::get);
        if old == 0 || tier(old) != tier(width) {
            WIDTH.with(|c| c.set(width));
            let narrow = width < NARROW_WIDTH;
            if narrow {
                w.add_css_class("narrow");
            } else {
                w.remove_css_class("narrow");
            }
            apply_narrow();
        } else {
            WIDTH.with(|c| c.set(width));
        }
        glib::ControlFlow::Continue
    });
    // Coming back to the window is when an edited working copy can be offered back.
    window.connect_is_active_notify(|w| {
        if w.is_active()
            && let Some(ui) = ui()
        {
            ui.browse.check_opened();
        }
    });

    UI.with(|u| {
        *u.borrow_mut() = Some(Rc::new(Ui { window: window.clone(), stack, overlay, progress, home, browse, create, cancel: RefCell::new(None) }))
    });
    install_keys(&window);
    window.connect_close_request(|_| {
        prefs::flush();
        if let Some(ui) = ui()
            && let Some(c) = ui.cancel.borrow().as_ref()
        {
            c.cancel();
        }
        glib::Propagation::Proceed
    });
}

fn install_keys(window: &gtk::ApplicationWindow) {
    let controller = gtk::ShortcutController::new();
    controller.set_scope(gtk::ShortcutScope::Global);
    let add = |accel: &str, f: fn()| {
        if let Some(trigger) = gtk::ShortcutTrigger::parse_string(accel) {
            let action = gtk::CallbackAction::new(move |_, _| {
                f();
                glib::Propagation::Stop
            });
            controller.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
        }
    };
    add("<Control>o", || {
        if let Some(ui) = ui() {
            ui.home.choose_archive();
        }
    });
    add("<Control>n", || {
        if let Some(ui) = ui() {
            ui.create.start(Vec::new());
        }
    });
    add("<Control>q", || {
        if let Some(w) = window_now() {
            w.close();
        }
    });
    add("<Control>w", || {
        if let Some(w) = window_now() {
            w.close();
        }
    });
    add("<Control>f", || {
        if let Some(ui) = ui()
            && current() == "browse"
        {
            ui.browse.focus_search();
        }
    });
    add("<Control>e", || {
        if let Some(ui) = ui()
            && current() == "browse"
        {
            ui.browse.extract_default();
        }
    });
    add("<Control>comma", || crate::views::settings::show());
    add("F1", || crate::views::settings::show_help());
    window.add_controller(controller);

    // Keys that must not steal typing from entries use the bubble phase.
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|_, key, _, state| {
        let Some(ui) = ui() else { return glib::Propagation::Proceed };
        let ctrl = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let view = current();
        match key {
            gtk::gdk::Key::Escape => {
                if view == "browse" {
                    ui.browse.escape();
                } else if view == "create" {
                    show("home");
                }
                glib::Propagation::Stop
            }
            gtk::gdk::Key::BackSpace if view == "browse" => {
                ui.browse.up();
                glib::Propagation::Stop
            }
            gtk::gdk::Key::a if ctrl && view == "browse" => {
                ui.browse.select_all();
                glib::Propagation::Stop
            }
            gtk::gdk::Key::Menu if view == "browse" => {
                ui.browse.menu_from_keyboard();
                glib::Propagation::Stop
            }
            gtk::gdk::Key::F10 if view == "browse" && state.contains(gtk::gdk::ModifierType::SHIFT_MASK) => {
                ui.browse.menu_from_keyboard();
                glib::Propagation::Stop
            }
            gtk::gdk::Key::question => {
                crate::views::settings::show_help();
                glib::Propagation::Stop
            }
            gtk::gdk::Key::Delete if view == "browse" => {
                ui.browse.delete_selected();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    window.add_controller(keys);
}

fn window_now() -> Option<gtk::ApplicationWindow> {
    window()
}

// ---------- Progress ----------

fn build_progress() -> Progress {
    let root = widgets::vbox(8);
    root.add_css_class("progress-card");
    root.set_halign(gtk::Align::Center);
    root.set_valign(gtk::Align::End);
    root.set_visible(false);

    let top = widgets::hbox(12);
    let title = widgets::label("", "progress-title");
    title.set_hexpand(true);
    let readout = widgets::label("", "value-readout");
    readout.add_css_class("dim");
    let cancel = gtk::Button::with_label("Cancel");
    cancel.connect_clicked(|_| {
        if let Some(ui) = ui()
            && let Some(c) = ui.cancel.borrow().as_ref()
        {
            c.cancel();
        }
    });
    top.append(&title);
    top.append(&readout);
    top.append(&cancel);
    root.append(&top);

    let bar = gtk::ProgressBar::new();
    bar.add_css_class("job-bar");
    root.append(&bar);

    let bottom = widgets::hbox(12);
    let current = widgets::label("", "mono");
    current.add_css_class("dim");
    current.add_css_class("progress-file");
    current.set_hexpand(true);
    current.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    bottom.append(&current);
    let open_when_done = gtk::CheckButton::with_label("Open folder when done");
    open_when_done.add_css_class("dim");
    open_when_done.set_active(prefs::get().open_when_done);
    open_when_done.connect_toggled(|c| {
        let on = c.is_active();
        prefs::update(|p| p.open_when_done = on);
    });
    bottom.append(&open_when_done);
    root.append(&bottom);
    Progress { root, open_when_done, title, current, readout, bar }
}

pub fn busy() -> bool {
    ui().is_some_and(|u| u.cancel.borrow().is_some())
}

/// What the progress card needs to know beyond the operation itself.
#[derive(Default)]
pub struct JobInfo {
    /// Bytes the job reads or writes, for speed and time left (0 when unknown).
    pub total: u64,
    /// The folder the result lands in, for "Open folder when done".
    pub folder: Option<PathBuf>,
}

fn clock(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Run a 7-Zip job with the progress card showing. `done` gets the outcome on the UI thread.
pub fn run_job(op: Op, info: JobInfo, done: impl FnOnce(Outcome) + 'static) {
    let Some(ui) = ui() else { return };
    let Some(binary) = sevenzip::binary() else {
        toast("7-Zip isn't installed. Install the 7zip package and try again.");
        return;
    };
    if busy() {
        toast("Another job is still running.");
        return;
    }
    let p = &ui.progress;
    p.title.set_text(op.title());
    p.current.set_text("");
    p.readout.set_text("");
    p.bar.set_fraction(0.0);
    p.bar.pulse();
    p.open_when_done.set_visible(info.folder.is_some());
    p.root.set_visible(true);
    let (rx, cancel) = job::run(binary, op);
    *ui.cancel.borrow_mut() = Some(cancel);
    let started = Instant::now();
    glib::spawn_future_local(async move {
        let mut done = Some(done);
        while let Ok(event) = rx.recv().await {
            match event {
                Event::Progress(pr) => {
                    let p = &ui.progress;
                    p.bar.set_fraction(f64::from(pr.percent) / 100.0);
                    let elapsed = started.elapsed().as_secs_f64();
                    let mut parts = vec![format!("{}%", pr.percent)];
                    // Speed and time left only once there's enough to go on.
                    if pr.percent >= 2 && elapsed >= 1.0 {
                        if info.total > 0 {
                            let rate = info.total as f64 * f64::from(pr.percent) / 100.0 / elapsed;
                            parts.push(format!("{}/s", crate::fmt::size(rate as u64)));
                        }
                        let left = elapsed * f64::from(100 - pr.percent.min(100)) / f64::from(pr.percent);
                        parts.push(format!("{} left", clock(left.round() as u64)));
                    } else {
                        parts.push(clock(elapsed as u64));
                    }
                    p.readout.set_text(&parts.join(" · "));
                    if !pr.current.is_empty() {
                        p.current.set_text(&pr.current);
                    }
                }
                Event::Done(outcome) => {
                    ui.progress.root.set_visible(false);
                    *ui.cancel.borrow_mut() = None;
                    if outcome == Outcome::Ok
                        && prefs::get().open_when_done
                        && let Some(folder) = &info.folder
                    {
                        crate::cmd::spawn(&["xdg-open", &folder.to_string_lossy()]);
                    }
                    if let Some(done) = done.take() {
                        done(outcome);
                    }
                    break;
                }
            }
        }
    });
}

/// The message for a job that didn't finish. Wrong passwords are the caller's to handle.
pub fn report_failure(outcome: &Outcome) {
    match outcome {
        Outcome::Ok => {}
        Outcome::Cancelled => toast("Cancelled."),
        Outcome::WrongPassword => toast("That password didn't work."),
        Outcome::Failed(m) => {
            let short = job::summary(m);
            if short.trim() == m.trim() {
                toast(&short);
            } else {
                let full = m.clone();
                toast_action(&short, "Details", move || show_details("7-Zip couldn't finish", &full));
            }
        }
    }
}

/// A dialog with 7-Zip's full output, scrollable and selectable.
pub fn show_details(title: &str, text: &str) {
    let (dialog, card) = widgets::dialog(title, 560);
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_monospace(true);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.add_css_class("code-block");
    view.buffer().set_text(text);
    let scroll = gtk::ScrolledWindow::builder().child(&view).min_content_height(220).max_content_height(420).propagate_natural_height(true).build();
    card.append(&scroll);
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    card.append(&close);
    dialog.present();
}

// ---------- Toasts ----------

pub fn toast(message: &str) {
    toast_with(message, None);
}

/// A toast with one action button, such as "Show in folder".
pub fn toast_action(message: &str, label: &str, act: impl Fn() + 'static) {
    toast_with(message, Some((label.to_string(), Box::new(act))));
}

type ToastAction = Option<(String, Box<dyn Fn()>)>;

fn toast_with(message: &str, action: ToastAction) {
    let Some(ui) = ui() else {
        eprintln!("archive: {message}");
        return;
    };
    // A new toast replaces the one showing.
    if let Some(old) = TOAST.with(|t| t.borrow_mut().take()) {
        ui.overlay.remove_overlay(&old);
    }
    let bx = widgets::hbox(14);
    bx.add_css_class("toast");
    let label = gtk::Label::new(Some(message));
    label.set_wrap(true);
    label.set_max_width_chars(64);
    label.set_selectable(false);
    bx.append(&label);
    if let Some((text, act)) = action {
        let b = gtk::Button::with_label(&text);
        b.add_css_class("flat");
        b.add_css_class("toast-action");
        let weak = bx.downgrade();
        b.connect_clicked(move |_| {
            act();
            if let Some(bx) = weak.upgrade()
                && let Some(parent) = bx.parent().and_downcast::<gtk::Overlay>()
            {
                parent.remove_overlay(&bx);
            }
        });
        bx.append(&b);
    }
    bx.set_halign(gtk::Align::Center);
    bx.set_valign(gtk::Align::End);
    ui.overlay.add_overlay(&bx);
    TOAST.with(|t| *t.borrow_mut() = Some(bx.clone()));
    let overlay = ui.overlay.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(if message.len() > 80 { 6000 } else { 3500 }), move || {
        if bx.parent().is_some() {
            overlay.remove_overlay(&bx);
        }
    });
}

// ---------- Dialogs and pickers ----------

fn start_dir() -> gio::File {
    let last = prefs::get().last_dir;
    let dir = if !last.is_empty() && Path::new(&last).is_dir() { PathBuf::from(last) } else { paths::home() };
    gio::File::for_path(dir)
}

fn remember_dir(file: &Path) {
    let dir = if file.is_dir() { file } else { file.parent().unwrap_or(file) };
    let dir = dir.to_string_lossy().to_string();
    prefs::update(|p| p.last_dir = dir);
}

fn to_paths(list: &gio::ListModel) -> Vec<PathBuf> {
    list.iter::<gio::File>().flatten().filter_map(|f| f.path()).collect()
}

/// Pick one or more files.
pub fn choose_files(title: &str, archives_only: bool, multiple: bool, done: impl Fn(Vec<PathBuf>) + 'static) {
    let dialog = gtk::FileDialog::builder().title(title).initial_folder(&start_dir()).build();
    if archives_only {
        let archives = gtk::FileFilter::new();
        archives.set_name(Some("Archives"));
        for ext in ["7z", "zip", "rar", "tar", "gz", "tgz", "xz", "txz", "bz2", "tbz2", "zst", "iso", "cab", "001", "wim", "lz4", "lz", "deb", "rpm", "jar", "apk"] {
            archives.add_suffix(ext);
        }
        let all = gtk::FileFilter::new();
        all.set_name(Some("All files"));
        all.add_pattern("*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&archives);
        filters.append(&all);
        dialog.set_filters(Some(&filters));
    }
    let parent = window();
    let finish = move |files: Vec<PathBuf>| {
        if let Some(first) = files.first() {
            remember_dir(first);
            done(files);
        }
    };
    if multiple {
        dialog.open_multiple(parent.as_ref(), gio::Cancellable::NONE, move |res| {
            if let Ok(list) = res {
                finish(to_paths(&list));
            }
        });
    } else {
        dialog.open(parent.as_ref(), gio::Cancellable::NONE, move |res| {
            if let Some(path) = res.ok().and_then(|f| f.path()) {
                finish(vec![path]);
            }
        });
    }
}

pub fn choose_folder(title: &str, done: impl Fn(PathBuf) + 'static) {
    let dialog = gtk::FileDialog::builder().title(title).initial_folder(&start_dir()).build();
    dialog.select_folder(window().as_ref(), gio::Cancellable::NONE, move |res| {
        if let Some(path) = res.ok().and_then(|f| f.path()) {
            remember_dir(&path);
            done(path);
        }
    });
}

/// "Not now" first, then the action. `danger` gives the action the destructive style.
pub fn confirm(title: &str, desc: &str, action: &str, danger: bool, on_ok: impl Fn() + 'static) {
    let (dialog, card) = widgets::dialog(title, 440);
    let d = widgets::label(desc, "dim");
    d.set_wrap(true);
    card.append(&d);
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(8);
    let cancel = gtk::Button::with_label("Not now");
    let ok = gtk::Button::with_label(action);
    ok.add_css_class(if danger { "destructive-action" } else { "suggested-action" });
    buttons.append(&cancel);
    buttons.append(&ok);
    card.append(&buttons);
    let dd = dialog.clone();
    cancel.connect_clicked(move |_| dd.close());
    let dd = dialog.clone();
    ok.connect_clicked(move |_| {
        dd.close();
        on_ok();
    });
    dialog.present();
    cancel.grab_focus();
}

/// Ask what to do about files that already exist.
pub fn ask_overwrite(dest: &Path, clashes: usize, on_choice: impl Fn(job::Overwrite) + 'static) {
    let (dialog, card) = widgets::dialog("Files already exist", 460);
    let what = if clashes == 1 { "One file is".to_string() } else { format!("{} files are", crate::fmt::thousands(clashes)) };
    let d = widgets::label(&format!("{what} already in {}. What should happen to them?", paths::pretty(dest)), "dim");
    d.set_wrap(true);
    card.append(&d);
    let on_choice = Rc::new(on_choice);
    let list = widgets::vbox(8);
    for (id, title, desc) in [
        (job::Overwrite::Replace, "Replace them", "The extracted files win."),
        (job::Overwrite::Skip, "Keep the existing ones", "Only files that aren't there yet are extracted."),
        (job::Overwrite::Rename, "Keep both", "Extracted files get a new name."),
    ] {
        let b = gtk::Button::new();
        b.add_css_class("choice-button");
        let c = widgets::vbox(2);
        c.append(&widgets::label(title, "settings-option-title"));
        c.append(&widgets::label(desc, "settings-option-description"));
        b.set_child(Some(&c));
        let (dd, cb) = (dialog.clone(), on_choice.clone());
        b.connect_clicked(move |_| {
            dd.close();
            cb(id);
        });
        list.append(&b);
    }
    card.append(&list);
    let cancel = gtk::Button::with_label("Not now");
    cancel.set_halign(gtk::Align::End);
    let dd = dialog.clone();
    cancel.connect_clicked(move |_| dd.close());
    card.append(&cancel);
    dialog.present();
}

/// Ask for a password. `on_ok` runs with the typed text.
pub fn ask_password(title: &str, desc: &str, on_ok: impl Fn(String) + 'static) {
    let (dialog, card) = widgets::dialog(title, 420);
    let d = widgets::label(desc, "dim");
    d.set_wrap(true);
    card.append(&d);
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_activates_default(false);
    card.append(&entry);
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let cancel = gtk::Button::with_label("Not now");
    let ok = gtk::Button::with_label("Unlock");
    ok.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&ok);
    card.append(&buttons);
    let dd = dialog.clone();
    cancel.connect_clicked(move |_| dd.close());
    let on_ok = Rc::new(on_ok);
    let submit = {
        let (dd, e) = (dialog.clone(), entry.clone());
        move || {
            let text = e.text().to_string();
            if text.is_empty() {
                return;
            }
            dd.close();
            on_ok(text);
        }
    };
    let s2 = submit.clone();
    ok.connect_clicked(move |_| s2());
    entry.connect_activate(move |_| submit());
    dialog.present();
    entry.grab_focus();
}

/// Open a folder in the file manager, selecting nothing.
pub fn show_in_folder(path: &Path) {
    let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
    crate::cmd::spawn(&["xdg-open", &dir.to_string_lossy()]);
}

/// Open an archive in the browse view.
pub fn open_archive(path: PathBuf) {
    if let Some(ui) = ui() {
        ui.browse.open(path);
    }
}

/// Go to the create view with these files already added.
pub fn start_create(files: Vec<PathBuf>) {
    if let Some(ui) = ui() {
        ui.create.start(files);
    }
}
