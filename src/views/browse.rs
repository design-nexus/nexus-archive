//! Browsing an archive: breadcrumbs, a sortable file list and the main actions.

use crate::sevenzip::job::{Op, Outcome, Overwrite};
use crate::sevenzip::parse::{Entry, Listing};
use crate::sevenzip::{self, ListError};
use crate::{cmd, fmt, paths, prefs, recent, widgets, window};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Size,
    Packed,
    Modified,
}

struct State {
    /// What the user opened.
    source: PathBuf,
    /// What is listed: the same file, or the tar inside a .tar.gz.
    archive: PathBuf,
    listing: Listing,
    password: Option<String>,
    cwd: String,
    sort: SortKey,
    ascending: bool,
    query: String,
    /// The entries behind the rows currently shown, in row order.
    shown: Vec<Entry>,
}

pub struct Browse {
    pub root: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    crumbs: gtk::Box,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    empty: gtk::Box,
    empty_title: gtk::Label,
    scroll: gtk::ScrolledWindow,
    sort_buttons: Vec<(SortKey, gtk::Button)>,
    info_body: gtk::Box,
    selection: gtk::Label,
    extract: gtk::Button,
    add: gtk::Button,
    test: gtk::Button,
    delete: gtk::Button,
    state: RefCell<State>,
}

fn name_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    // Numbers sort by value, so "file2" comes before "file10".
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let (mut n, mut m) = (String::new(), String::new());
                while let Some(&c) = x.peek().filter(|c| c.is_ascii_digit()) {
                    n.push(c);
                    x.next();
                }
                while let Some(&d) = y.peek().filter(|d| d.is_ascii_digit()) {
                    m.push(d);
                    y.next();
                }
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let ord = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            (Some(c), Some(d)) => {
                let ord = c.to_lowercase().cmp(d.to_lowercase());
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
                x.next();
                y.next();
            }
        }
    }
}

fn icon_for(e: &Entry) -> &'static str {
    if e.is_dir {
        return "folder-symbolic";
    }
    let ext = e.name().rsplit_once('.').map(|(_, x)| x.to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif" => "image-x-generic-symbolic",
        "mp3" | "flac" | "ogg" | "wav" | "opus" | "m4a" => "audio-x-generic-symbolic",
        "mp4" | "mkv" | "webm" | "avi" | "mov" => "video-x-generic-symbolic",
        "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "zst" | "bz2" => "package-x-generic-symbolic",
        "sh" | "py" | "rs" | "js" | "c" | "h" | "cpp" | "go" | "html" | "css" | "json" | "toml" | "yml" | "yaml" => "text-x-script-symbolic",
        _ => "text-x-generic-symbolic",
    }
}

impl Browse {
    pub fn new() -> Rc<Self> {
        let root = widgets::vbox(0);
        root.add_css_class("browse");

        // ----- Header -----
        let header = widgets::hbox(10);
        header.add_css_class("browse-header");
        let back = widgets::icon_button("go-previous-symbolic", "Back to start");
        back.connect_clicked(|_| window::show("home"));
        header.append(&back);
        let text = widgets::vbox(0);
        text.set_hexpand(true);
        let title = widgets::label("", "browse-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let subtitle = widgets::label("", "dim");
        subtitle.add_css_class("mono");
        subtitle.add_css_class("browse-subtitle");
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&title);
        text.append(&subtitle);
        header.append(&text);

        let extract = widgets::tool_button("document-save-symbolic", "Extract", "Extract (Ctrl+E)");
        extract.add_css_class("suggested-action");
        let extract_more = gtk::MenuButton::new();
        extract_more.set_icon_name("pan-down-symbolic");
        extract_more.add_css_class("extract-more");
        extract_more.set_tooltip_text(Some("Extract to…"));
        let add = widgets::tool_button("list-add-symbolic", "Add", "Add files to this archive");
        let test = widgets::tool_button("emblem-ok-symbolic", "Test", "Check the archive for errors");
        let delete = widgets::tool_button("edit-delete-symbolic", "Delete", "Delete the selection from the archive (Delete)");
        let info_toggle = gtk::ToggleButton::new();
        info_toggle.set_icon_name("dialog-information-symbolic");
        info_toggle.add_css_class("flat");
        info_toggle.set_tooltip_text(Some("Archive details"));
        info_toggle.set_valign(gtk::Align::Center);
        let gear = widgets::icon_button("emblem-system-symbolic", "Settings (Ctrl+,)");
        gear.connect_clicked(|_| crate::views::settings::show());

        let split = widgets::hbox(0);
        split.add_css_class("split-button");
        split.append(&extract);
        split.append(&extract_more);
        header.append(&split);
        header.append(&add);
        header.append(&test);
        header.append(&delete);
        header.append(&info_toggle);
        header.append(&gear);
        root.append(&header);

        // ----- Info card -----
        let info_body = widgets::hbox(24);
        info_body.add_css_class("info-card");
        let info = gtk::Revealer::new();
        info.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        info.set_child(Some(&info_body));
        let i2 = info.clone();
        info_toggle.connect_toggled(move |b| i2.set_reveal_child(b.is_active()));
        root.append(&info);

        // ----- Path and search -----
        let bar = widgets::hbox(10);
        bar.add_css_class("path-bar");
        let crumbs = widgets::hbox(2);
        let crumb_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .child(&crumbs)
            .hexpand(true)
            .build();
        let crumb_adj = crumb_scroll.hadjustment();
        crumb_adj.connect_upper_notify(|a| a.set_value(a.upper() - a.page_size()));
        bar.append(&crumb_scroll);
        let search = gtk::SearchEntry::new();
        search.add_css_class("settings-search");
        search.set_placeholder_text(Some("Search this archive"));
        search.set_width_chars(22);
        search.set_max_width_chars(28);
        bar.append(&search);
        root.append(&bar);

        // ----- Column headings and the list -----
        let cols = widgets::hbox(10);
        cols.add_css_class("file-head");
        let mut sort_buttons = Vec::new();
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_size_request(22, 1);
        cols.append(&spacer);
        for (key, label, class) in [
            (SortKey::Name, "Name", "col-name"),
            (SortKey::Size, "Size", "col-size"),
            (SortKey::Packed, "Packed", "col-packed"),
            (SortKey::Modified, "Modified", "col-modified"),
        ] {
            let b = gtk::Button::with_label(label);
            b.add_css_class("flat");
            b.add_css_class("col-head");
            b.add_css_class(class);
            b.set_hexpand(key == SortKey::Name);
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(if key == SortKey::Name { 0.0 } else { 1.0 });
            }
            sort_buttons.push((key, b.clone()));
            cols.append(&b);
        }
        root.append(&cols);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Multiple);
        list.set_activate_on_single_click(false);
        list.add_css_class("file-list");
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&list)
            .vexpand(true)
            .build();
        let empty_title = widgets::label("", "empty-title");
        let empty = widgets::vbox(8);
        empty.set_valign(gtk::Align::Center);
        empty.set_halign(gtk::Align::Center);
        empty.set_vexpand(true);
        empty.append(&empty_title);
        empty.set_visible(false);
        root.append(&scroll);
        root.append(&empty);

        // ----- Status line -----
        let status = widgets::hbox(10);
        status.add_css_class("status-bar");
        let selection = widgets::label("", "dim");
        selection.add_css_class("mono");
        selection.set_hexpand(true);
        status.append(&selection);
        root.append(&status);

        let this = Rc::new(Browse {
            root: root.clone(),
            title,
            subtitle,
            crumbs,
            search: search.clone(),
            list: list.clone(),
            empty,
            empty_title,
            scroll,
            sort_buttons,
            info_body,
            selection,
            extract: extract.clone(),
            add: add.clone(),
            test: test.clone(),
            delete: delete.clone(),
            state: RefCell::new(State {
                source: PathBuf::new(),
                archive: PathBuf::new(),
                listing: Listing::default(),
                password: None,
                cwd: String::new(),
                sort: SortKey::Name,
                ascending: true,
                query: String::new(),
                shown: Vec::new(),
            }),
        });
        this.wire(&extract_more);
        // Dropping files onto an open archive adds them.
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let t = this.clone();
        target.connect_drop(move |_, value, _, _| {
            let files = crate::views::home::dropped(value);
            if !files.is_empty() {
                t.add_files(files);
            }
            true
        });
        root.add_controller(target);
        this
    }

    fn wire(self: &Rc<Self>, extract_more: &gtk::MenuButton) {
        for (key, b) in &self.sort_buttons {
            let (t, key) = (self.clone(), *key);
            b.connect_clicked(move |_| {
                {
                    let mut s = t.state.borrow_mut();
                    if s.sort == key {
                        s.ascending = !s.ascending;
                    } else {
                        s.sort = key;
                        s.ascending = key == SortKey::Name;
                    }
                }
                t.refresh();
            });
        }
        let t = self.clone();
        self.search.connect_search_changed(move |e| {
            t.state.borrow_mut().query = e.text().trim().to_lowercase();
            t.refresh();
        });
        let t = self.clone();
        self.list.connect_row_activated(move |_, row| t.activate(row.index() as usize));
        let t = self.clone();
        self.list.connect_selected_rows_changed(move |_| t.update_status());
        let t = self.clone();
        self.extract.connect_clicked(move |_| t.extract_default());
        let t = self.clone();
        self.add.connect_clicked(move |_| {
            window::choose_files("Add files", false, true, {
                let t = t.clone();
                move |files| t.add_files(files)
            });
        });
        let t = self.clone();
        self.test.connect_clicked(move |_| t.test_archive());
        let t = self.clone();
        self.delete.connect_clicked(move |_| t.delete_selected());

        let popover = gtk::Popover::new();
        popover.add_css_class("menu-popover");
        let menu = widgets::vbox(2);
        let item = |label: &str| {
            let b = gtk::Button::with_label(label);
            b.add_css_class("flat");
            b.add_css_class("menu-item");
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            b
        };
        let choose = item("Extract to…");
        let (t, pop) = (self.clone(), popover.clone());
        choose.connect_clicked(move |_| {
            pop.popdown();
            let t = t.clone();
            window::choose_folder("Extract to", move |dir| t.extract_to(dir, false));
        });
        let here = item("Extract here, without a new folder");
        let (t, pop) = (self.clone(), popover.clone());
        here.connect_clicked(move |_| {
            pop.popdown();
            let dir = t.state.borrow().source.parent().map(Path::to_path_buf).unwrap_or_default();
            t.extract_to(dir, false);
        });
        menu.append(&choose);
        menu.append(&here);
        popover.set_child(Some(&menu));
        extract_more.set_popover(Some(&popover));
    }

    // ---------- Opening ----------

    pub fn open(self: &Rc<Self>, path: PathBuf) {
        self.open_with(path, None);
    }

    fn open_with(self: &Rc<Self>, path: PathBuf, password: Option<String>) {
        let Some(binary) = sevenzip::binary() else {
            window::toast("7-Zip isn't installed. Install the 7zip package and try again.");
            return;
        };
        if !path.is_file() {
            window::toast(&format!("{} isn't a file.", paths::pretty(&path)));
            return;
        }
        let this = self.clone();
        let (p, pw) = (path.clone(), password.clone());
        cmd::background(move || sevenzip::list(binary, &p, pw.as_deref()), move |result| match result {
            Ok(listing) => {
                if listing.entries.is_empty() && listing.info.kind.is_empty() {
                    window::toast("That doesn't look like an archive 7-Zip can open.");
                    return;
                }
                if listing.is_tar_wrapper() {
                    this.open_inner(path, listing, password);
                } else {
                    this.finish_open(path.clone(), path, listing, password);
                }
            }
            Err(ListError::NeedsPassword) => {
                let asked = password.is_some();
                let this = this.clone();
                let path2 = path.clone();
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                window::ask_password(
                    if asked { "Wrong password" } else { "This archive is locked" },
                    &format!("Enter the password for {name}."),
                    move |pw| this.open_with(path2.clone(), Some(pw)),
                );
            }
            Err(ListError::Failed(m)) => window::toast(&m),
        });
    }

    /// A .tar.gz holds one tar: unpack that to the cache and list it, so the real files show.
    fn open_inner(self: &Rc<Self>, source: PathBuf, wrapper: Listing, password: Option<String>) {
        let Some(binary) = sevenzip::binary() else { return };
        let dest = paths::open_dir().join(format!("{:x}-inner", path_hash(&source)));
        let _ = std::fs::remove_dir_all(&dest);
        let tar = dest.join(&wrapper.entries[0].path);
        let op = Op::Extract { archive: source.clone(), dest, paths: vec![], password: password.clone(), overwrite: Overwrite::Replace };
        let this = self.clone();
        window::run_job(op, move |out| {
            if out != Outcome::Ok {
                window::report_failure(&out);
                return;
            }
            let t = tar.clone();
            cmd::background(move || sevenzip::list(binary, &t, None), move |result| match result {
                Ok(listing) => this.finish_open(source, tar, listing, password),
                Err(_) => window::toast("The tar inside couldn't be read."),
            });
        });
    }

    fn finish_open(self: &Rc<Self>, source: PathBuf, archive: PathBuf, listing: Listing, password: Option<String>) {
        recent::record(&source);
        {
            let mut s = self.state.borrow_mut();
            s.source = source;
            s.archive = archive;
            s.listing = listing;
            s.password = password;
            s.cwd.clear();
            s.query.clear();
        }
        self.search.set_text("");
        self.fill_header();
        self.refresh();
        window::show("browse");
    }

    /// Archives that are a stream plus a tar can't be edited in place.
    fn editable(&self) -> bool {
        let s = self.state.borrow();
        let ok = s.source == s.archive;
        if !ok {
            window::toast("This archive can't be changed. Extract it and make a new one instead.");
        }
        ok
    }

    fn fill_header(&self) {
        let s = self.state.borrow();
        let name = s.source.file_name().unwrap_or_default().to_string_lossy().to_string();
        self.title.set_text(&name);
        let l = &s.listing;
        let kind = if l.info.kind.is_empty() { "Archive".to_string() } else { l.info.kind.to_uppercase() };
        self.subtitle.set_text(&format!(
            "{kind} · {} · {}",
            fmt::count(l.file_count(), "file", "files"),
            fmt::size(l.total_size())
        ));
        while let Some(c) = self.info_body.first_child() {
            self.info_body.remove(&c);
        }
        let physical = if l.info.physical_size > 0 { l.info.physical_size } else { std::fs::metadata(&s.archive).map(|m| m.len()).unwrap_or(0) };
        let ratio = if l.total_size() > 0 { format!("{:.0}%", physical as f64 / l.total_size() as f64 * 100.0) } else { "—".into() };
        let facts: Vec<(&str, String)> = vec![
            ("Format", kind),
            ("Archive size", fmt::size(physical)),
            ("Unpacked", fmt::size(l.total_size())),
            ("Ratio", ratio),
            ("Method", if l.info.method.is_empty() { "—".into() } else { l.info.method.clone() }),
            ("Encrypted", if l.info.headers_encrypted { "Names too".into() } else if l.encrypted() { "Yes".into() } else { "No".into() }),
        ];
        for (k, v) in facts {
            let c = widgets::vbox(2);
            c.append(&widgets::label(&k.to_uppercase(), "info-key"));
            let val = widgets::label(&v, "mono");
            val.set_ellipsize(gtk::pango::EllipsizeMode::End);
            c.append(&val);
            self.info_body.append(&c);
        }
    }

    // ---------- Listing ----------

    fn refresh(self: &Rc<Self>) {
        let (rows, cwd, query) = {
            let mut s = self.state.borrow_mut();
            let mut rows: Vec<Entry> = if s.query.is_empty() {
                s.listing.children(&s.cwd)
            } else {
                s.listing.entries.iter().filter(|e| e.path.to_lowercase().contains(&s.query)).cloned().collect()
            };
            let (key, asc) = (s.sort, s.ascending);
            rows.sort_by(|a, b| {
                // Folders stay on top whatever the sort.
                let ord = b.is_dir.cmp(&a.is_dir).then_with(|| {
                    let o = match key {
                        SortKey::Name => name_cmp(a.name(), b.name()),
                        SortKey::Size => a.size.cmp(&b.size),
                        SortKey::Packed => a.packed.unwrap_or(0).cmp(&b.packed.unwrap_or(0)),
                        SortKey::Modified => a.modified.cmp(&b.modified),
                    };
                    if asc { o } else { o.reverse() }
                });
                ord.then_with(|| name_cmp(a.name(), b.name()))
            });
            s.shown = rows.clone();
            (rows, s.cwd.clone(), s.query.clone())
        };
        self.rebuild_crumbs(&cwd);
        for (key, b) in &self.sort_buttons {
            let s = self.state.borrow();
            let base = match key {
                SortKey::Name => "Name",
                SortKey::Size => "Size",
                SortKey::Packed => "Packed",
                SortKey::Modified => "Modified",
            };
            let arrow = if s.sort == *key { if s.ascending { " ↑" } else { " ↓" } } else { "" };
            b.set_label(&format!("{base}{arrow}"));
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(if *key == SortKey::Name { 0.0 } else { 1.0 });
            }
        }
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let searching = !query.is_empty();
        for e in &rows {
            self.list.append(&file_row(e, searching));
        }
        let none = rows.is_empty();
        self.scroll.set_visible(!none);
        self.empty.set_visible(none);
        self.empty_title.set_text(&if searching { format!("Nothing matches “{query}”") } else { "This folder is empty".to_string() });
        window::apply_narrow();
        self.update_status();
        let adj = self.scroll.vadjustment();
        adj.set_value(0.0);
    }

    fn rebuild_crumbs(self: &Rc<Self>, cwd: &str) {
        while let Some(c) = self.crumbs.first_child() {
            self.crumbs.remove(&c);
        }
        let root_name = {
            let s = self.state.borrow();
            s.source.file_name().unwrap_or_default().to_string_lossy().to_string()
        };
        let _ = root_name;
        let mut parts: Vec<(String, String)> = vec![("All files".to_string(), String::new())];
        let mut acc = String::new();
        for seg in cwd.split('/').filter(|s| !s.is_empty()) {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            parts.push((seg.to_string(), acc.clone()));
        }
        let last = parts.len() - 1;
        for (i, (label, path)) in parts.into_iter().enumerate() {
            if i > 0 {
                self.crumbs.append(&widgets::label("›", "dim"));
            }
            let b = gtk::Button::with_label(&label);
            b.add_css_class("flat");
            b.add_css_class("crumb");
            if i == last {
                b.add_css_class("current");
            }
            let t = self.clone();
            b.connect_clicked(move |_| t.go(&path));
            self.crumbs.append(&b);
        }
    }

    fn go(self: &Rc<Self>, dir: &str) {
        {
            let mut s = self.state.borrow_mut();
            s.cwd = dir.to_string();
            s.query.clear();
        }
        self.search.set_text("");
        self.refresh();
    }

    pub fn up(self: &Rc<Self>) {
        let parent = {
            let s = self.state.borrow();
            if s.cwd.is_empty() { return } else { s.cwd.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default() }
        };
        self.go(&parent);
    }

    pub fn escape(self: &Rc<Self>) {
        if !self.state.borrow().query.is_empty() {
            self.search.set_text("");
        } else if self.list.selected_rows().is_empty() {
            window::show("home");
        } else {
            self.list.unselect_all();
        }
    }

    pub fn focus_search(&self) {
        self.search.grab_focus();
    }

    pub fn select_all(&self) {
        self.list.select_all();
    }

    fn selected(&self) -> Vec<Entry> {
        let s = self.state.borrow();
        self.list.selected_rows().iter().filter_map(|r| s.shown.get(r.index() as usize).cloned()).collect()
    }

    fn update_status(&self) {
        let sel = self.selected();
        let s = self.state.borrow();
        let text = if sel.is_empty() {
            format!("{} here", fmt::count(s.shown.len(), "item", "items"))
        } else {
            let bytes: u64 = sel.iter().map(|e| e.size).sum();
            format!("{} selected · {}", sel.len(), fmt::size(bytes))
        };
        self.selection.set_text(&text);
        self.delete.set_sensitive(!sel.is_empty());
        self.extract.set_tooltip_text(Some(if sel.is_empty() { "Extract everything (Ctrl+E)" } else { "Extract the selection (Ctrl+E)" }));
    }

    fn activate(self: &Rc<Self>, index: usize) {
        let Some(e) = self.state.borrow().shown.get(index).cloned() else { return };
        if e.is_dir {
            self.go(&e.path);
            return;
        }
        // A file opens from a working copy, so the archive itself is never touched.
        let (archive, password) = {
            let s = self.state.borrow();
            (s.archive.clone(), s.password.clone())
        };
        let dest = paths::open_dir().join(format!("{:x}", path_hash(&archive)));
        let op = Op::Extract { archive, dest: dest.clone(), paths: vec![e.path.clone()], password, overwrite: Overwrite::Replace };
        let this = self.clone();
        window::run_job(op, move |out| match out {
            Outcome::Ok => cmd::spawn(&["xdg-open", &dest.join(&e.path).to_string_lossy()]),
            Outcome::WrongPassword => this.ask_extract_password(),
            other => window::report_failure(&other),
        });
    }

    // ---------- Actions ----------

    fn ask_extract_password(self: &Rc<Self>) {
        let this = self.clone();
        window::ask_password("Password needed", "Some files in this archive are encrypted.", move |pw| {
            this.state.borrow_mut().password = Some(pw);
            window::toast("Password set. Try again.");
        });
    }

    /// Extract the selection (or everything) to the default place.
    pub fn extract_default(self: &Rc<Self>) {
        let p = prefs::get();
        let archive = self.state.borrow().source.clone();
        let base = if p.extract_dir.is_empty() { archive.parent().map(Path::to_path_buf).unwrap_or_default() } else { PathBuf::from(&p.extract_dir) };
        self.extract_to(base, p.extract_into_folder);
    }

    fn extract_to(self: &Rc<Self>, base: PathBuf, new_folder: bool) {
        let (archive, password, encrypted) = {
            let s = self.state.borrow();
            (s.archive.clone(), s.password.clone(), s.listing.encrypted())
        };
        if encrypted && password.is_none() {
            let this = self.clone();
            window::ask_password("Password needed", "Some files in this archive are encrypted.", move |pw| {
                this.state.borrow_mut().password = Some(pw);
                this.extract_to(base.clone(), new_folder);
            });
            return;
        }
        let stem = archive_stem(&self.state.borrow().source);
        let dest = if new_folder { base.join(&stem) } else { base };
        let paths: Vec<String> = self.selected().into_iter().map(|e| e.path).collect();
        let exists = dest.is_dir() && std::fs::read_dir(&dest).map(|mut d| d.next().is_some()).unwrap_or(false);
        let go = {
            let this = self.clone();
            let dest = dest.clone();
            move |overwrite: Overwrite| {
                let op = Op::Extract { archive: archive.clone(), dest: dest.clone(), paths: paths.clone(), password: password.clone(), overwrite };
                let (this, dest) = (this.clone(), dest.clone());
                window::run_job(op, move |out| match out {
                    Outcome::Ok => {
                        let d = dest.clone();
                        window::toast_action(&format!("Extracted to {}", paths::pretty(&dest)), "Show in folder", move || window::show_in_folder(&d));
                    }
                    Outcome::WrongPassword => {
                        this.state.borrow_mut().password = None;
                        this.ask_extract_password();
                    }
                    other => window::report_failure(&other),
                });
            }
        };
        match Overwrite::from_id(&prefs::get().overwrite) {
            Overwrite::Ask if exists => window::ask_overwrite(&dest, go),
            Overwrite::Ask => go(Overwrite::Replace),
            fixed => go(fixed),
        }
    }

    fn add_files(self: &Rc<Self>, files: Vec<PathBuf>) {
        if !self.editable() {
            return;
        }
        let (archive, password) = {
            let s = self.state.borrow();
            (s.archive.clone(), s.password.clone())
        };
        let n = files.len();
        let this = self.clone();
        window::run_job(Op::Add { archive: archive.clone(), inputs: files, password }, move |out| match out {
            Outcome::Ok => {
                window::toast(&format!("Added {}.", fmt::count(n, "item", "items")));
                this.reload();
            }
            other => window::report_failure(&other),
        });
    }

    pub fn delete_selected(self: &Rc<Self>) {
        let sel = self.selected();
        if sel.is_empty() || !self.editable() {
            return;
        }
        let (archive, password) = {
            let s = self.state.borrow();
            (s.archive.clone(), s.password.clone())
        };
        let n = sel.len();
        let paths: Vec<String> = sel.into_iter().map(|e| e.path).collect();
        let this = self.clone();
        window::confirm(
            &format!("Delete {} from the archive?", fmt::count(n, "item", "items")),
            "They are removed from the archive file itself. Your other files aren't touched.",
            "Delete",
            true,
            move || {
                let this = this.clone();
                window::run_job(Op::Delete { archive: archive.clone(), paths: paths.clone(), password: password.clone() }, move |out| match out {
                    Outcome::Ok => {
                        window::toast("Deleted from the archive.");
                        this.reload();
                    }
                    other => window::report_failure(&other),
                });
            },
        );
    }

    fn test_archive(self: &Rc<Self>) {
        let (archive, password) = {
            let s = self.state.borrow();
            (s.archive.clone(), s.password.clone())
        };
        window::run_job(Op::Test { archive, password }, |out| match out {
            Outcome::Ok => window::toast("No errors found. The archive is healthy."),
            other => window::report_failure(&other),
        });
    }

    /// Re-read the archive after it changed, keeping the folder open.
    fn reload(self: &Rc<Self>) {
        let Some(binary) = sevenzip::binary() else { return };
        let (archive, password) = {
            let s = self.state.borrow();
            (s.archive.clone(), s.password.clone())
        };
        let this = self.clone();
        cmd::background(move || sevenzip::list(binary, &archive, password.as_deref()), move |result| {
            if let Ok(listing) = result {
                {
                    let mut s = this.state.borrow_mut();
                    s.listing = listing;
                    // The folder may be gone after a delete.
                    let cwd = s.cwd.clone();
                    if !cwd.is_empty() && s.listing.children(&cwd).is_empty() && !s.listing.entries.iter().any(|e| e.path == cwd) {
                        s.cwd.clear();
                    }
                }
                this.fill_header();
                this.refresh();
            }
        });
    }
}

fn path_hash(p: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    h.finish()
}

/// `photos.tar.gz` → `photos`.
pub fn archive_stem(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    for ext in [".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".7z.001", ".zip.001"] {
        if let Some(s) = name.strip_suffix(ext) {
            return s.to_string();
        }
    }
    path.file_stem().unwrap_or_default().to_string_lossy().to_string()
}

fn file_row(e: &Entry, show_path: bool) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("file-row");
    let c = widgets::hbox(10);
    let icon = gtk::Image::from_icon_name(icon_for(e));
    if e.is_dir {
        icon.add_css_class("accent-text");
    } else {
        icon.add_css_class("dim");
    }
    c.append(&icon);
    let name_box = widgets::vbox(0);
    name_box.set_hexpand(true);
    name_box.set_valign(gtk::Align::Center);
    let name = widgets::label(e.name(), "file-name");
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    name_box.append(&name);
    if show_path && !e.parent().is_empty() {
        let p = widgets::label(e.parent(), "mono");
        p.add_css_class("dim");
        p.add_css_class("file-path");
        p.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        name_box.append(&p);
    }
    c.append(&name_box);
    if e.encrypted {
        let lock = gtk::Image::from_icon_name("changes-prevent-symbolic");
        lock.add_css_class("dim");
        lock.set_tooltip_text(Some("Encrypted"));
        c.append(&lock);
    }
    let cell = |text: String, class: &str| {
        let l = widgets::label(&text, "mono");
        l.add_css_class("dim");
        l.add_css_class("cell");
        l.add_css_class(class);
        l.set_xalign(1.0);
        l
    };
    let size = if e.is_dir && e.size == 0 { String::new() } else { fmt::size(e.size) };
    c.append(&cell(size, "col-size"));
    c.append(&cell(e.packed.map(fmt::size).unwrap_or_default(), "col-packed"));
    c.append(&cell(e.modified.clone(), "col-modified"));
    row.set_child(Some(&c));
    let _ = glib::Priority::DEFAULT;
    row
}
