# Nexus Archive

An archive manager for [Omarchy](https://omarchy.org), built on 7-Zip. Open an archive to
browse it, extract all or part of it, or compress files into a new one. It takes its
colours from your Omarchy theme and fits a half-screen tile.

## What it does

- **Browse:** folders with a breadcrumb path, sortable columns (name, size, packed size,
  modified), and a search that looks through the whole archive. Double-click a file to open
  it from a working copy; the archive itself isn't touched.
- **Extract:** everything or just the selection, into a new folder named after the archive
  or straight into a folder you pick. It asks what to do about files that already exist,
  or follows the choice you made in settings.
- **Create:** 7z, zip, tar.gz or tar.xz, with five compression levels. 7z and zip can be
  password-protected (7z can hide file names too), split into parts, and 7z can be solid.
- **Change an archive:** add files by dragging them onto it, delete entries from it, and
  test it for errors. `.tar.gz` and `.tar.xz` open as the tar inside and can't be changed.
- **Formats:** anything the installed 7-Zip can read, including rar, iso, cab, deb and rpm.
- **Passwords:** locked archives ask for the password when opened or extracted. Passwords
  are kept in memory only while the archive is open.
- **Progress:** a card at the bottom shows the current file, percent and elapsed time,
  with a Cancel button. Cancelled or failed jobs leave no half-written archive.

Passwords are passed to 7-Zip on its command line, so other users on the same machine could
see them in the process list while a job runs.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/design-nexus/nexus-archive/main/install.sh | bash
```

This installs GTK 4 and 7-Zip if they're missing, builds with Cargo, and installs to
`~/.local`. Add `--default` to also make it the app that opens archive files:

```sh
curl -fsSL https://raw.githubusercontent.com/design-nexus/nexus-archive/main/install.sh | bash -s -- --default
```

Remove it with `./uninstall.sh` (add `--purge` to delete its settings too).

## Command line

```sh
archive FILE            # open an archive
archive --create FILE…  # compress these files into a new archive
```

## Keyboard

| Keys | Action |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>O</kbd> | Open an archive |
| <kbd>Ctrl</kbd>+<kbd>N</kbd> | New archive |
| <kbd>Ctrl</kbd>+<kbd>E</kbd> | Extract the selection, or everything |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | Search the archive |
| <kbd>Ctrl</kbd>+<kbd>A</kbd> | Select all |
| <kbd>Enter</kbd> | Open the folder or file |
| <kbd>Backspace</kbd> | Up one folder |
| <kbd>Delete</kbd> | Delete from the archive |
| <kbd>Esc</kbd> | Clear the search, unselect, or go back |
| <kbd>Ctrl</kbd>+<kbd>,</kbd> | Settings |
| <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Close |

## Settings

Open them with the gear. **Archive window** has *Follow Omarchy theme*, a theme list
(15 bundled themes, plus your own in `~/.config/nexus-archive/themes/*.toml`), *Glow* and
*Reduce motion*. Extracting has the default folder and the overwrite choice. They're saved in
`~/.config/nexus-archive/settings.toml`.

## Build

```sh
cargo build --release
cargo test
```

Needs GTK 4 (4.18 or newer) and `7zz` or `7z` on `PATH` (the tests that run 7-Zip are
skipped without it).

## License

MIT
