# Nexus Archive

GTK4 (gtk4-rs, no libadwaita) archive manager wrapping the `7zz`/`7z` command line.

- `src/sevenzip/parse.rs`: `7z l -slt` parser and folder tree. `job.rs`: command builders (`plan`), the runner (`run`, progress and cancel) and the tests, including real round trips that skip without 7-Zip.
- `src/views/{home,browse,create,settings}.rs`: the three views and the settings dialog. `src/window.rs`: window, progress card, toasts, dialogs, file pickers.
- `src/theme.rs`, `style.css`: semantic colour tokens, 15 themes, Follow Omarchy (polled each second).
- 7-Zip redraws progress with backspaces, not carriage returns; the runner splits on both.
- Dev: `NARC_APP_ID=io.github.design_nexus.Dev ./target/debug/archive FILE` runs a separate instance.
- Single-stream formats (gz, xz, bz2) list one nameless entry; the parser names it after the archive. A `.tar.*` opens as the inner tar, extracted to the cache (`Browse::open_inner`).
