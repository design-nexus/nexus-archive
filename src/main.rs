//! Nexus Archive: a 7-Zip archive manager for Omarchy.

mod clamp;
mod cmd;
mod fmt;
mod paths;
mod prefs;
mod recent;
mod settings_dialog;
mod sevenzip;
mod theme;
mod views;
mod widgets;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};
use std::path::PathBuf;

pub const APP_ID: &str = "io.github.design_nexus.Archive";

const USAGE: &str = "Usage: archive [OPTIONS] [FILE…]\n\
\n\
  FILE            open this archive\n\
  --create FILE…  compress these files into a new archive\n";

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return glib::ExitCode::SUCCESS;
    }
    // GL renders only on the GPU in use; Vulkan would wake a sleeping discrete GPU.
    if std::env::var_os("GSK_RENDERER").is_none() {
        // SAFETY: still single-threaded; nothing else reads the environment yet.
        unsafe { std::env::set_var("GSK_RENDERER", "ngl") };
    }
    // Developer aid: a different id gives a separate instance for test runs.
    let id = std::env::var("NARC_APP_ID").unwrap_or_else(|_| APP_ID.to_string());
    let app = gtk::Application::builder().application_id(id).flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE).build();
    app.connect_command_line(|app, cl| {
        let argv: Vec<String> = cl.arguments().iter().map(|a| a.to_string_lossy().to_string()).collect();
        let create = argv.iter().any(|a| a == "--create");
        let files: Vec<PathBuf> = argv
            .iter()
            .skip(1)
            .filter(|a| !a.starts_with("--"))
            // Relative paths resolve against the caller's directory, and URIs work too.
            .filter_map(|a| cl.create_file_for_arg(a).path())
            .collect();
        window::present(app, window::Request { files, create });
        glib::ExitCode::SUCCESS
    });
    app.connect_shutdown(|_| prefs::flush());
    app.run()
}
