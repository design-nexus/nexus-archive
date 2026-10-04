//! Making a new archive: pick files, then format, level, name and options.

use crate::sevenzip::job::{CreateOpts, Format, Op, Outcome};
use crate::{cmd, fmt, paths, prefs, widgets, window};
use gtk::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

struct State {
    inputs: Vec<PathBuf>,
    format: Format,
    level: u8,
    name: String,
    name_touched: bool,
    dir: PathBuf,
    /// The user picked the folder, so adding files no longer moves it.
    dir_touched: bool,
    password: String,
    encrypt_names: bool,
    solid: bool,
    split_mb: u32,
    threads: u32,
}

pub struct Create {
    pub root: gtk::Box,
    files: gtk::Box,
    files_summary: gtk::Label,
    empty_hint: gtk::Label,
    name: gtk::Entry,
    ext: gtk::Label,
    dir_button: gtk::Button,
    password_row: gtk::Box,
    names_row: gtk::Box,
    solid_row: gtk::Box,
    split_row: gtk::Box,
    tar_note: gtk::Label,
    summary: gtk::Label,
    create: gtk::Button,
    state: RefCell<State>,
}

fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    std::fs::read_dir(path).map(|d| d.flatten().map(|e| dir_size(&e.path())).sum()).unwrap_or(0)
}

impl Create {
    pub fn new() -> Rc<Self> {
        let p = prefs::get();
        let format = Format::from_id(&p.last_format);
        let level = if [0, 1, 5, 7, 9].contains(&p.last_level) { p.last_level } else { 5 };

        let root = widgets::vbox(0);
        root.add_css_class("create");
        let body = widgets::vbox(0);
        body.add_css_class("settings-page");

        // ----- Header -----
        let header = widgets::hbox(10);
        header.add_css_class("section-header");
        let back = widgets::icon_button("go-previous-symbolic", "Back to start");
        back.connect_clicked(|_| window::show("home"));
        header.append(&back);
        let text = widgets::vbox(0);
        text.set_hexpand(true);
        text.append(&widgets::label("New archive", "section-title"));
        text.append(&widgets::label("Choose what goes in, then how it should be packed.", "section-description"));
        header.append(&text);
        body.append(&header);

        // ----- Files -----
        let g = widgets::vbox(0);
        let head = widgets::hbox(8);
        let t = widgets::label("FILES", "group-title");
        t.set_hexpand(true);
        head.append(&t);
        let files_summary = widgets::label("", "mono");
        files_summary.add_css_class("dim");
        files_summary.set_margin_top(24);
        head.append(&files_summary);
        g.append(&head);
        let card = widgets::vbox(0);
        card.add_css_class("files-card");
        let files = widgets::vbox(0);
        card.append(&files);
        let empty_hint = widgets::label("Drop files or folders here, or add them with the buttons below.", "dim");
        empty_hint.add_css_class("files-empty");
        empty_hint.set_wrap(true);
        empty_hint.set_justify(gtk::Justification::Center);
        card.append(&empty_hint);
        let buttons = widgets::hbox(8);
        buttons.add_css_class("files-actions");
        let add_files = widgets::labeled_button("list-add-symbolic", "Add files");
        let add_folder = widgets::labeled_button("folder-new-symbolic", "Add folder");
        buttons.append(&add_files);
        buttons.append(&add_folder);
        card.append(&buttons);
        g.append(&card);
        body.append(&g);

        // ----- Options -----
        let g = widgets::vbox(0);
        g.append(&widgets::label("OPTIONS", "group-title"));
        let list = widgets::vbox(6);
        let format_opts: Vec<(String, String)> = Format::ALL.iter().map(|f| (f.id().to_string(), f.id().to_string())).collect();
        let tar_note = widgets::label("tar.gz and tar.xz can't be password-protected, so that option is hidden.", "settings-option-description");
        tar_note.set_wrap(true);
        let level_opts = widgets::opts(&[("0", "Store"), ("1", "Fast"), ("5", "Normal"), ("7", "Maximum"), ("9", "Ultra")]);

        let name = gtk::Entry::new();
        name.set_hexpand(true);
        name.set_width_chars(16);
        let ext = widgets::label("", "mono");
        ext.add_css_class("dim");
        let name_box = widgets::hbox(6);
        name_box.append(&name);
        name_box.append(&ext);
        name_box.set_hexpand(true);
        let name_row = widgets::row("Name", "", Some(name_box.upcast_ref()));
        let dir_button = gtk::Button::new();
        dir_button.add_css_class("path-button");
        let dir_row = widgets::row("Save in", "", Some(dir_button.upcast_ref()));

        let this_format = format;
        let this_level = level;
        // The rows are wired to `this` once it exists; the controls are made here.
        let (password_row, password_entry) = {
            let e = gtk::PasswordEntry::new();
            e.set_show_peek_icon(true);
            e.set_width_chars(18);
            let r = widgets::row("Password", "Anyone opening the archive will need it.", Some(e.upcast_ref()));
            (r, e)
        };
        let (names_row, names_switch) = widgets::switch_row("Hide file names", "Names are encrypted too (7z only).", false, |_| {});
        let (solid_row, solid_switch) = widgets::switch_row("Solid archive", "Smaller for many similar files, but slower to open one file.", true, |_| {});
        let split_opts = widgets::opts(&[("0", "Off"), ("100", "100 MB"), ("650", "650 MB (CD)"), ("700", "700 MB"), ("1024", "1 GB"), ("4000", "4 GB (FAT32)")]);
        let (split_row, split_dd) = widgets::choice_row("Split into parts", "Make several smaller files.", split_opts.clone(), "0", |_| {});
        let thread_opts = widgets::opts(&[("0", "Automatic"), ("1", "1"), ("2", "2"), ("4", "4"), ("8", "8")]);
        let (threads_row, threads_dd) = widgets::choice_row("Processor threads", "Fewer threads leave the computer responsive.", thread_opts.clone(), "0", |_| {});

        let state = State {
            inputs: Vec::new(),
            format: this_format,
            level: this_level,
            name: String::new(),
            name_touched: false,
            dir: paths::home(),
            dir_touched: false,
            password: String::new(),
            encrypt_names: false,
            solid: true,
            split_mb: 0,
            threads: 0,
        };

        let summary = widgets::label("", "mono");
        summary.add_css_class("dim");
        summary.set_hexpand(true);
        summary.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let create = widgets::labeled_button("package-x-generic-symbolic", "Create archive");
        create.add_css_class("suggested-action");

        let this = Rc::new(Create {
            root: root.clone(),
            files: files.clone(),
            files_summary,
            empty_hint,
            name: name.clone(),
            ext: ext.clone(),
            dir_button: dir_button.clone(),
            password_row: password_row.clone(),
            names_row: names_row.clone(),
            solid_row: solid_row.clone(),
            split_row: split_row.clone(),
            tar_note: tar_note.clone(),
            summary: summary.clone(),
            create: create.clone(),
            state: RefCell::new(state),
        });

        // Format and level controls need `this` in their callbacks.
        let t = this.clone();
        let (format_row, _) = {
            let seg = widgets::segmented(&format_opts, this_format.id(), move |id| {
                let f = Format::from_id(&id);
                t.state.borrow_mut().format = f;
                prefs::update(|p| p.last_format = id);
                t.sync();
            });
            (widgets::row("Format", "7z packs smallest; zip opens everywhere.", Some(seg.upcast_ref())), seg)
        };
        let t = this.clone();
        let level_row = widgets::segmented_row("Compression", "Higher levels are smaller but slower.", level_opts, &this_level.to_string(), move |id| {
            let level: u8 = id.parse().unwrap_or(5);
            t.state.borrow_mut().level = level;
            prefs::update(|p| p.last_level = level);
            t.sync();
        });

        list.append(&format_row);
        list.append(&tar_note);
        list.append(&level_row);
        list.append(&name_row);
        list.append(&dir_row);
        g.append(&list);
        body.append(&g);

        // ----- Advanced (folded) -----
        let adv = widgets::vbox(0);
        adv.set_margin_top(14);
        let toggle = gtk::Button::new();
        toggle.add_css_class("disclosure");
        let tc = widgets::hbox(12);
        let tt = widgets::vbox(2);
        tt.set_hexpand(true);
        tt.append(&widgets::label("Advanced", "settings-option-title"));
        tt.append(&widgets::label("Password, split parts, solid mode, threads", "settings-option-description"));
        tc.append(&tt);
        let chevron = gtk::Image::from_icon_name("pan-end-symbolic");
        chevron.add_css_class("chevron");
        tc.append(&chevron);
        toggle.set_child(Some(&tc));
        adv.append(&toggle);
        let rev = gtk::Revealer::new();
        rev.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        let inner = widgets::vbox(6);
        inner.set_margin_top(6);
        for r in [&password_row, &names_row, &solid_row, &split_row, &threads_row] {
            inner.append(r);
        }
        rev.set_child(Some(&inner));
        adv.append(&rev);
        let (rv, ch) = (rev.clone(), chevron.clone());
        toggle.connect_clicked(move |_| {
            let open = !rv.reveals_child();
            rv.set_reveal_child(open);
            if open {
                ch.add_css_class("open");
            } else {
                ch.remove_css_class("open");
            }
        });
        body.append(&adv);

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&body)
            .vexpand(true)
            .build();
        widgets::center_clamp(&scroll, &body, 920);
        root.append(&scroll);

        let footer = widgets::hbox(12);
        footer.add_css_class("create-footer");
        footer.append(&summary);
        footer.append(&create);
        root.append(&footer);

        // ----- Wiring -----
        let t = this.clone();
        name.connect_changed(move |e| {
            let mut s = t.state.borrow_mut();
            // Text that already matches the state was set by the program, not typed.
            if s.name != e.text() {
                s.name = e.text().to_string();
                s.name_touched = true;
            }
            drop(s);
            t.update_summary();
        });
        let t = this.clone();
        dir_button.connect_clicked(move |_| {
            let t = t.clone();
            window::choose_folder("Save the archive in", move |dir| {
                {
                    let mut s = t.state.borrow_mut();
                    s.dir = dir;
                    s.dir_touched = true;
                }
                t.sync();
            });
        });
        let t = this.clone();
        password_entry.connect_changed(move |e| {
            t.state.borrow_mut().password = e.text().to_string();
            t.update_summary();
        });
        let t = this.clone();
        names_switch.connect_active_notify(move |s| t.state.borrow_mut().encrypt_names = s.is_active());
        let t = this.clone();
        solid_switch.connect_active_notify(move |s| t.state.borrow_mut().solid = s.is_active());
        let t = this.clone();
        split_dd.connect_selected_notify(move |d| {
            if let Some((id, _)) = split_opts.get(d.selected() as usize) {
                t.state.borrow_mut().split_mb = id.parse().unwrap_or(0);
            }
        });
        let t = this.clone();
        threads_dd.connect_selected_notify(move |d| {
            if let Some((id, _)) = thread_opts.get(d.selected() as usize) {
                t.state.borrow_mut().threads = id.parse().unwrap_or(0);
            }
        });
        let t = this.clone();
        add_files.connect_clicked(move |_| {
            let t = t.clone();
            window::choose_files("Add files", false, true, move |f| t.add_inputs(f));
        });
        let t = this.clone();
        add_folder.connect_clicked(move |_| {
            let t = t.clone();
            window::choose_folder("Add a folder", move |f| t.add_inputs(vec![f]));
        });
        let t = this.clone();
        create.connect_clicked(move |_| t.run());

        let target = gtk::DropTarget::new(gtk::gdk::FileList::static_type(), gtk::gdk::DragAction::COPY);
        let t = this.clone();
        target.connect_drop(move |_, value, _, _| {
            t.add_inputs(crate::views::home::dropped(value));
            true
        });
        root.add_controller(target);

        this.rebuild_files();
        this.sync();
        this
    }

    /// Open the view, optionally with files already chosen.
    pub fn start(self: &Rc<Self>, files: Vec<PathBuf>) {
        if !files.is_empty() {
            self.add_inputs(files);
        }
        window::show("create");
    }

    pub fn add_inputs(self: &Rc<Self>, files: Vec<PathBuf>) {
        {
            let mut s = self.state.borrow_mut();
            for f in files {
                if f.exists() && !s.inputs.contains(&f) {
                    s.inputs.push(f);
                }
            }
            if !s.dir_touched
                && let Some(parent) = s.inputs.first().and_then(|p| p.parent())
            {
                s.dir = parent.to_path_buf();
            }
            if !s.name_touched || s.name.is_empty() {
                s.name = default_name(&s.inputs);
            }
        }
        self.rebuild_files();
        self.sync();
        self.measure();
    }

    fn remove_input(self: &Rc<Self>, path: &Path) {
        {
            let mut s = self.state.borrow_mut();
            s.inputs.retain(|p| p != path);
            if !s.name_touched {
                s.name = default_name(&s.inputs);
            }
        }
        self.rebuild_files();
        self.sync();
        self.measure();
    }

    /// Add up the sizes off the UI thread: folders can be large.
    fn measure(self: &Rc<Self>) {
        let inputs = self.state.borrow().inputs.clone();
        let n = inputs.len();
        if n == 0 {
            self.files_summary.set_text("");
            return;
        }
        let this = self.clone();
        cmd::background(
            move || inputs.iter().map(|p| dir_size(p)).sum::<u64>(),
            move |total| {
                if this.state.borrow().inputs.len() == n {
                    this.files_summary.set_text(&format!("{} · {}", fmt::count(n, "item", "items"), fmt::size(total)));
                }
            },
        );
    }

    fn rebuild_files(self: &Rc<Self>) {
        while let Some(c) = self.files.first_child() {
            self.files.remove(&c);
        }
        let inputs = self.state.borrow().inputs.clone();
        self.empty_hint.set_visible(inputs.is_empty());
        for path in inputs {
            let row = widgets::hbox(10);
            row.add_css_class("input-row");
            let icon = gtk::Image::from_icon_name(if path.is_dir() { "folder-symbolic" } else { "text-x-generic-symbolic" });
            icon.add_css_class(if path.is_dir() { "accent-text" } else { "dim" });
            row.append(&icon);
            let name = widgets::label(&path.file_name().unwrap_or_default().to_string_lossy(), "file-name");
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            name.set_hexpand(true);
            row.append(&name);
            let dir = widgets::label(&path.parent().map(paths::pretty).unwrap_or_default(), "mono");
            dir.add_css_class("dim");
            dir.add_css_class("col-hide-narrow");
            dir.set_xalign(1.0);
            dir.set_max_width_chars(40);
            dir.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            row.append(&dir);
            let remove = widgets::icon_button("window-close-symbolic", "Remove");
            let (t, p) = (self.clone(), path.clone());
            remove.connect_clicked(move |_| t.remove_input(&p));
            row.append(&remove);
            self.files.append(&row);
        }
        window::apply_narrow();
    }

    /// Bring every dependent control in line with the state.
    fn sync(self: &Rc<Self>) {
        let (format, dir, name) = {
            let s = self.state.borrow();
            (s.format, s.dir.clone(), s.name.clone())
        };
        self.ext.set_text(&format!(".{}", format.extension()));
        if self.name.text() != name {
            self.name.set_text(&name);
        }
        self.dir_button.set_label(&paths::pretty(&dir));
        if let Some(l) = self.dir_button.child().and_downcast::<gtk::Label>() {
            l.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            l.add_css_class("mono");
        }
        self.password_row.set_visible(format.can_encrypt());
        self.names_row.set_visible(format.can_encrypt_names());
        self.solid_row.set_visible(format.can_solid());
        self.split_row.set_visible(matches!(format, Format::SevenZ | Format::Zip));
        self.tar_note.set_visible(!format.can_encrypt());
        self.update_summary();
    }

    fn update_summary(&self) {
        let s = self.state.borrow();
        let ready = !s.inputs.is_empty() && !s.name.trim().is_empty();
        self.create.set_sensitive(ready);
        let text = if s.inputs.is_empty() {
            "Add at least one file or folder.".to_string()
        } else {
            let lock = if s.format.can_encrypt() && !s.password.is_empty() { " · password" } else { "" };
            format!("{}{lock}", paths::pretty(&output_path(&s)))
        };
        self.summary.set_text(&text);
    }

    fn run(self: &Rc<Self>) {
        let (opts, output) = {
            let s = self.state.borrow();
            let output = output_path(&s);
            let opts = CreateOpts {
                format: s.format,
                level: s.level,
                output: output.clone(),
                inputs: s.inputs.clone(),
                password: Some(s.password.clone()).filter(|p| !p.is_empty()),
                encrypt_names: s.encrypt_names,
                solid: s.solid,
                split_mb: Some(s.split_mb).filter(|m| *m > 0),
                threads: Some(s.threads).filter(|t| *t > 0),
            };
            (opts, output)
        };
        let go = {
            let this = self.clone();
            move || {
                let (opts, output) = (opts.clone(), output.clone());
                let this = this.clone();
                let split = opts.split_mb.is_some();
                let original: u64 = opts.inputs.iter().map(|p| dir_size(p)).sum();
                window::run_job(Op::Create(opts), move |out| match out {
                    Outcome::Ok => {
                        let size = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
                        let ratio = if original > 0 && !split { format!(" ({:.0}% of the original)", size as f64 / original as f64 * 100.0) } else { String::new() };
                        let o = output.clone();
                        window::toast_action(
                            &format!("Created {}{}", output.file_name().unwrap_or_default().to_string_lossy(), if split { String::new() } else { format!(" · {}{ratio}", fmt::size(size)) }),
                            "Show in folder",
                            move || window::show_in_folder(&o),
                        );
                        {
                            let mut s = this.state.borrow_mut();
                            s.inputs.clear();
                            s.name.clear();
                            s.name_touched = false;
                        }
                        this.rebuild_files();
                        this.files_summary.set_text("");
                        this.sync();
                        if split {
                            window::show("home");
                        } else {
                            window::open_archive(output);
                        }
                    }
                    other => window::report_failure(&other),
                });
            }
        };
        let existing = self.state.borrow().split_mb == 0 && output_path(&self.state.borrow()).exists();
        if existing {
            let p = output_path(&self.state.borrow());
            window::confirm("Replace the existing file?", &format!("{} already exists. Creating the archive replaces it.", paths::pretty(&p)), "Replace", true, go);
        } else {
            go();
        }
    }
}

fn output_path(s: &State) -> PathBuf {
    s.dir.join(format!("{}.{}", s.name.trim(), s.format.extension()))
}

fn default_name(inputs: &[PathBuf]) -> String {
    match inputs {
        [] => String::new(),
        [one] => one.file_stem().or_else(|| one.file_name()).unwrap_or_default().to_string_lossy().to_string(),
        _ => "Archive".to_string(),
    }
}
