//! Everything that talks to the `7zz` command-line tool.

pub mod job;
pub mod parse;

use crate::cmd;

/// The first 7-Zip binary found on `PATH`, if any.
pub fn binary() -> Option<&'static str> {
    ["7zz", "7z", "7za"].into_iter().find(|b| cmd::present(b))
}

use parse::Listing;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq)]
pub enum ListError {
    /// The archive's file names are encrypted, or the password was wrong.
    NeedsPassword,
    Failed(String),
}

/// Read an archive's contents. Blocking: call it off the UI thread.
pub fn list(binary: &str, archive: &Path, password: Option<&str>) -> Result<Listing, ListError> {
    let mut cmd = Command::new(binary);
    cmd.args(["l", "-slt", "-sccUTF-8"]);
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        cmd.arg(format!("-p{p}"));
    }
    cmd.arg("--").arg(archive).stdin(Stdio::null());
    let out = cmd.output().map_err(|e| ListError::Failed(format!("Couldn't start {binary}: {e}")))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if out.status.success() || !text.contains("\n----------\n") && out.status.code() == Some(1) && text.contains("Warnings") {
        return Ok(parse::parse_listing(&text));
    }
    let err = format!("{}{}", String::from_utf8_lossy(&out.stderr), text);
    let lower = err.to_lowercase();
    if lower.contains("wrong password") || lower.contains("enter password") || lower.contains("encrypted archive") {
        return Err(ListError::NeedsPassword);
    }
    // A damaged archive often still lists what it can; show that rather than nothing.
    if text.contains("\n----------\n") {
        return Ok(parse::parse_listing(&text));
    }
    match job::classify_failure(&err) {
        job::Outcome::Failed(m) => Err(ListError::Failed(job::summary(&m))),
        _ => Err(ListError::NeedsPassword),
    }
}

/// File extensions that are worth offering to open as archives.
pub fn looks_like_archive(path: &Path) -> bool {
    const EXTS: &[&str] = &[
        "7z", "zip", "rar", "tar", "gz", "tgz", "xz", "txz", "bz2", "tbz2", "zst", "lz", "lz4", "iso", "cab", "arj", "lzh",
        "wim", "jar", "apk", "deb", "rpm", "cpio", "squashfs", "xar", "z", "001", "vhd", "img", "dmg", "epub",
    ];
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTS.contains(&e.to_lowercase().as_str()))
}
