//! Building `7zz` command lines and running them off the UI thread with progress.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overwrite {
    #[default]
    Ask,
    Skip,
    Replace,
    Rename,
}

impl Overwrite {
    pub fn from_id(id: &str) -> Self {
        match id {
            "skip" => Overwrite::Skip,
            "replace" => Overwrite::Replace,
            "rename" => Overwrite::Rename,
            _ => Overwrite::Ask,
        }
    }

    fn flag(self) -> &'static str {
        match self {
            // The UI resolves "ask" before the job starts, so it never reaches 7zz.
            Overwrite::Ask | Overwrite::Skip => "-aos",
            Overwrite::Replace => "-aoa",
            Overwrite::Rename => "-aou",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    SevenZ,
    Zip,
    TarGz,
    TarXz,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::SevenZ, Format::Zip, Format::TarGz, Format::TarXz];

    pub fn id(self) -> &'static str {
        match self {
            Format::SevenZ => "7z",
            Format::Zip => "zip",
            Format::TarGz => "tar.gz",
            Format::TarXz => "tar.xz",
        }
    }

    pub fn from_id(id: &str) -> Self {
        Self::ALL.into_iter().find(|f| f.id() == id).unwrap_or(Format::SevenZ)
    }

    pub fn extension(self) -> &'static str {
        self.id()
    }

    /// Passwords only work on formats that can encrypt.
    pub fn can_encrypt(self) -> bool {
        matches!(self, Format::SevenZ | Format::Zip)
    }

    /// Only 7z can hide the file names, and only 7z and zip can be solid or split sensibly.
    pub fn can_encrypt_names(self) -> bool {
        self == Format::SevenZ
    }

    pub fn can_solid(self) -> bool {
        self == Format::SevenZ
    }
}

#[derive(Debug, Clone)]
pub struct CreateOpts {
    pub format: Format,
    /// 0 (store) to 9 (ultra).
    pub level: u8,
    pub output: PathBuf,
    pub inputs: Vec<PathBuf>,
    pub password: Option<String>,
    pub encrypt_names: bool,
    pub solid: bool,
    /// Split into volumes of this many megabytes.
    pub split_mb: Option<u32>,
    pub threads: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum Op {
    Create(CreateOpts),
    Extract { archive: PathBuf, dest: PathBuf, paths: Vec<String>, password: Option<String>, overwrite: Overwrite },
    Add { archive: PathBuf, inputs: Vec<PathBuf>, password: Option<String> },
    /// Put files back at their own paths inside the archive: `rel` is relative to `base`.
    Update { archive: PathBuf, base: PathBuf, rel: Vec<String>, password: Option<String> },
    Delete { archive: PathBuf, paths: Vec<String>, password: Option<String> },
    Test { archive: PathBuf, password: Option<String> },
}

impl Op {
    pub fn title(&self) -> &'static str {
        match self {
            Op::Create(_) => "Creating archive",
            Op::Extract { .. } => "Extracting",
            Op::Add { .. } => "Adding files",
            Op::Update { .. } => "Updating the archive",
            Op::Delete { .. } => "Deleting",
            Op::Test { .. } => "Testing",
        }
    }
}

fn pw(arg: &Option<String>) -> Option<String> {
    arg.as_ref().filter(|p| !p.is_empty()).map(|p| format!("-p{p}"))
}

/// Switches that make `7zz` print progress and nothing else we have to parse.
const QUIET: [&str; 3] = ["-bsp1", "-bso0", "-bse1"];

pub fn level_switch(level: u8) -> String {
    format!("-mx={}", level.min(9))
}

/// One `7zz` invocation (arguments only, without the program).
pub type Stage = Vec<String>;

/// The stages to run in order, files to rename into place afterwards, and temp files to remove.
#[derive(Debug, Default)]
pub struct Plan {
    pub stages: Vec<Stage>,
    pub rename: Vec<(PathBuf, PathBuf)>,
    /// The folder 7-Zip runs in, so relative paths keep their place in the archive.
    pub cwd: Option<PathBuf>,
    /// Folders to create before the first stage.
    pub mkdir: Vec<PathBuf>,
    /// Temp files or folders to remove at the end, whatever happened.
    pub cleanup: Vec<PathBuf>,
}

fn s(x: impl Into<String>) -> String {
    x.into()
}

pub fn temp_name(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.part"))
}

pub fn plan(op: &Op) -> Plan {
    let mut p = Plan::default();
    match op {
        Op::Extract { archive, dest, paths, password, overwrite } => {
            let mut a = vec![s("x"), s("-y"), overwrite.flag().into(), format!("-o{}", dest.display())];
            a.extend(QUIET.map(s));
            a.extend(pw(password));
            // `--` so a name that starts with a dash isn't read as a switch.
            a.push(s("--"));
            a.push(archive.display().to_string());
            a.extend(paths.iter().cloned());
            p.stages.push(a);
        }
        Op::Test { archive, password } => {
            let mut a = vec![s("t")];
            a.extend(QUIET.map(s));
            a.extend(pw(password));
            a.push(s("--"));
            a.push(archive.display().to_string());
            p.stages.push(a);
        }
        Op::Delete { archive, paths, password } => {
            let mut a = vec![s("d")];
            a.extend(QUIET.map(s));
            a.extend(pw(password));
            a.push(s("--"));
            a.push(archive.display().to_string());
            a.extend(paths.iter().cloned());
            p.stages.push(a);
        }
        Op::Add { archive, inputs, password } => {
            let mut a = vec![s("a")];
            a.extend(QUIET.map(s));
            a.extend(pw(password));
            a.push(s("--"));
            a.push(archive.display().to_string());
            a.extend(inputs.iter().map(|i| i.display().to_string()));
            p.stages.push(a);
        }
        Op::Update { archive, base, rel, password } => {
            let mut a = vec![s("a")];
            a.extend(QUIET.map(s));
            a.extend(pw(password));
            a.push(s("--"));
            a.push(archive.display().to_string());
            a.extend(rel.iter().cloned());
            p.stages.push(a);
            p.cwd = Some(base.clone());
        }
        Op::Create(o) => plan_create(o, &mut p),
    }
    p
}

fn plan_create(o: &CreateOpts, p: &mut Plan) {
    let inputs: Vec<String> = o.inputs.iter().map(|i| i.display().to_string()).collect();
    let threads = o.threads.map(|t| format!("-mmt={t}"));
    match o.format {
        Format::TarGz | Format::TarXz => {
            // 7-Zip compresses a tar in two steps: tar first, then the compressor.
            // The compressor stores the tar's own file name, so it gets its real name
            // inside a private work folder.
            let name = o.output.file_name().unwrap_or_default().to_string_lossy().to_string();
            let work = o.output.with_file_name(format!(".{name}.work"));
            let stem = name.strip_suffix(".tar.gz").or_else(|| name.strip_suffix(".tar.xz")).unwrap_or(&name);
            let tar = work.join(format!("{stem}.tar"));
            p.mkdir.push(work.clone());
            let kind = if o.format == Format::TarGz { "gzip" } else { "xz" };
            let mut a = vec![s("a"), s("-ttar")];
            a.extend(QUIET.map(s));
            a.push(s("--"));
            a.push(tar.display().to_string());
            a.extend(inputs);
            p.stages.push(a);
            let mut b = vec![s("a"), format!("-t{kind}"), level_switch(o.level)];
            b.extend(threads);
            b.extend(QUIET.map(s));
            b.push(s("--"));
            let out = temp_name(&o.output);
            b.push(out.display().to_string());
            b.push(tar.display().to_string());
            p.stages.push(b);
            p.rename.push((out, o.output.clone()));
            p.cleanup.push(work);
        }
        Format::SevenZ | Format::Zip => {
            let kind = if o.format == Format::SevenZ { "7z" } else { "zip" };
            let split = o.split_mb.filter(|m| *m > 0);
            // A split archive is written under its final name: the volumes are numbered
            // by 7-Zip, so there is no single file to rename into place.
            let out = if split.is_some() { o.output.clone() } else { temp_name(&o.output) };
            let mut a = vec![s("a"), format!("-t{kind}"), level_switch(o.level)];
            if o.format.can_solid() {
                a.push(format!("-ms={}", if o.solid { "on" } else { "off" }));
            }
            a.extend(threads);
            if let Some(mb) = split {
                a.push(format!("-v{mb}m"));
            }
            if let Some(pass) = o.password.as_ref().filter(|p| !p.is_empty() && o.format.can_encrypt()) {
                a.push(format!("-p{pass}"));
                if o.format == Format::SevenZ && o.encrypt_names {
                    a.push(s("-mhe=on"));
                } else if o.format == Format::Zip {
                    a.push(s("-mem=AES256"));
                }
            }
            a.extend(QUIET.map(s));
            a.push(s("--"));
            a.push(out.display().to_string());
            a.extend(inputs);
            p.stages.push(a);
            if split.is_none() {
                p.rename.push((out, o.output.clone()));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    /// 0–100 across all stages.
    pub percent: u8,
    pub current: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Ok,
    Cancelled,
    WrongPassword,
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum Event {
    Progress(Progress),
    Done(Outcome),
}

/// Lets the UI stop a running job.
#[derive(Clone, Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
    pid: Arc<AtomicU32>,
}

impl Cancel {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        let pid = self.pid.load(Ordering::SeqCst);
        if pid != 0 {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
    }

    fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// Parse one progress line such as ` 42% 3 + dir/file.txt` into (percent, current file).
pub fn parse_progress(line: &str) -> Option<(u8, String)> {
    let line = line.trim_start();
    let (num, rest) = line.split_once('%')?;
    let percent: u8 = num.trim().parse().ok()?;
    let rest = rest.trim_start();
    // After the percent come an optional file counter and a "+" / "-" / "=" marker.
    let current = rest
        .split_once(" + ")
        .or_else(|| rest.split_once(" - "))
        .or_else(|| rest.split_once(" = "))
        .map(|(_, f)| f)
        .unwrap_or(rest);
    Some((percent.min(100), current.trim().to_string()))
}

/// Sort 7-Zip's error output: wrong passwords get their own outcome, and anything else
/// keeps the whole text so the UI can show it under "Details".
pub fn classify_failure(text: &str) -> Outcome {
    let lower = text.to_lowercase();
    if lower.contains("wrong password") || lower.contains("data error in encrypted file") {
        return Outcome::WrongPassword;
    }
    let text = text.trim();
    Outcome::Failed(if text.is_empty() { "7-Zip reported an error".to_string() } else { text.to_string() })
}

/// The one line of 7-Zip's output worth showing in a toast.
pub fn summary(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| l.starts_with("ERROR:") || l.contains("Can not open") || l.contains("Cannot open") || l.contains("No space"))
        .map(|l| l.trim_start_matches("ERROR:").trim().to_string())
        .or_else(|| text.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_string))
        .unwrap_or_else(|| "7-Zip reported an error".to_string())
}

/// Run an operation on a worker thread. Events arrive on the returned channel;
/// the last one is always `Event::Done`.
pub fn run(binary: &'static str, op: Op) -> (async_channel::Receiver<Event>, Cancel) {
    let (tx, rx) = async_channel::unbounded();
    let cancel = Cancel::default();
    let c = cancel.clone();
    std::thread::spawn(move || {
        let outcome = execute(binary, &op, &c, &tx);
        let _ = tx.send_blocking(Event::Done(outcome));
    });
    (rx, cancel)
}

/// Run an operation on this thread, without progress. For small, quick jobs only.
pub fn run_blocking(binary: &str, op: &Op) -> Outcome {
    let (tx, _rx) = async_channel::unbounded();
    execute(binary, op, &Cancel::default(), &tx)
}

fn execute(binary: &str, op: &Op, cancel: &Cancel, tx: &async_channel::Sender<Event>) -> Outcome {
    let plan = plan(op);
    let total = plan.stages.len().max(1);
    let mut outcome = Outcome::Ok;
    for dir in &plan.mkdir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return Outcome::Failed(format!("Couldn't create {}: {e}", dir.display()));
        }
    }
    for (i, stage) in plan.stages.iter().enumerate() {
        match run_stage(binary, stage, plan.cwd.as_deref(), cancel, |pr| {
            let percent = ((i * 100 + pr.percent as usize) / total) as u8;
            let _ = tx.send_blocking(Event::Progress(Progress { percent, current: pr.current }));
        }) {
            Outcome::Ok => {}
            other => {
                outcome = other;
                break;
            }
        }
    }
    if outcome == Outcome::Ok {
        for (from, to) in &plan.rename {
            if let Err(e) = std::fs::rename(from, to) {
                outcome = Outcome::Failed(format!("Couldn't write {}: {e}", to.display()));
                break;
            }
        }
    }
    if outcome != Outcome::Ok {
        // Never leave half-written archives lying around.
        for (from, _) in &plan.rename {
            let _ = std::fs::remove_file(from);
        }
    }
    for f in &plan.cleanup {
        let _ = std::fs::remove_dir_all(f).or_else(|_| std::fs::remove_file(f));
    }
    outcome
}

fn run_stage(binary: &str, args: &[String], cwd: Option<&Path>, cancel: &Cancel, mut on_progress: impl FnMut(Progress)) -> Outcome {
    if cancel.is_cancelled() {
        return Outcome::Cancelled;
    }
    let mut command = Command::new(binary);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return Outcome::Failed(format!("Couldn't start {binary}: {e}")),
    };
    cancel.pid.store(child.id(), Ordering::SeqCst);
    let mut stderr = child.stderr.take().expect("piped");
    let err_thread = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let mut stdout = child.stdout.take().expect("piped");
    let mut all = String::new();
    let mut line: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    let mut last = 255u8;
    while let Ok(n) = stdout.read(&mut buf) {
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            // Progress is redrawn in place with carriage returns and backspaces.
            if b == b'\r' || b == b'\n' || b == 0x08 {
                let text = String::from_utf8_lossy(&line).to_string();
                if let Some((percent, current)) = parse_progress(&text) {
                    if percent != last || !current.is_empty() {
                        last = percent;
                        on_progress(Progress { percent, current });
                    }
                } else if !text.trim().is_empty() && !text.contains(" Scan") {
                    all.push_str(&text);
                    all.push('\n');
                }
                line.clear();
            } else {
                line.push(b);
            }
        }
    }
    let status = child.wait();
    cancel.pid.store(0, Ordering::SeqCst);
    let mut errors = err_thread.join().unwrap_or_default();
    errors.push_str(&all);
    if cancel.is_cancelled() {
        return Outcome::Cancelled;
    }
    match status {
        Ok(st) if st.success() => Outcome::Ok,
        Ok(_) => classify_failure(&errors),
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sevenzip::{self, parse};

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("narc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn finish(op: Op) -> Outcome {
        let bin = sevenzip::binary().unwrap();
        let (rx, _) = run(bin, op);
        loop {
            if let Event::Done(o) = rx.recv_blocking().unwrap() {
                return o;
            }
        }
    }

    /// Create, list, test and extract with the real tool, then compare with the originals.
    #[test]
    fn round_trip_with_real_7zip() {
        let Some(bin) = sevenzip::binary() else { return };
        for (format, ext) in [(Format::SevenZ, "7z"), (Format::Zip, "zip"), (Format::TarGz, "tar.gz"), (Format::TarXz, "tar.xz")] {
            let dir = tmp(ext.replace('.', "-").as_str());
            let src = dir.join("src");
            std::fs::create_dir_all(src.join("sub dir")).unwrap();
            std::fs::write(src.join("a.txt"), "hello").unwrap();
            std::fs::write(src.join("sub dir/é ü.txt"), "unicode").unwrap();
            let output = dir.join(format!("out.{ext}"));
            let opts = CreateOpts { output: output.clone(), inputs: vec![src.clone()], ..create(format) };
            assert_eq!(finish(Op::Create(opts)), Outcome::Ok, "{ext} create");
            assert!(output.is_file() && !temp_name(&output).exists(), "{ext} left a temp file");
            let l = sevenzip::list(bin, &output, None).unwrap_or_else(|e| panic!("{ext} list: {e:?}"));
            assert!(!l.entries.is_empty(), "{ext} lists nothing");
            assert_eq!(finish(Op::Test { archive: output.clone(), password: None }), Outcome::Ok);
            let dest = dir.join("x");
            let ex = Op::Extract { archive: output.clone(), dest: dest.clone(), paths: vec![], password: None, overwrite: Overwrite::Replace };
            // tar.gz holds a tar that needs a second extraction.
            assert_eq!(finish(ex), Outcome::Ok, "{ext} extract");
            let root = if ext.starts_with("tar.") {
                let tar = dest.join("out.tar");
                assert!(tar.is_file(), "{ext} should hold out.tar, has {:?}", std::fs::read_dir(&dest).unwrap().flatten().map(|e| e.file_name()).collect::<Vec<_>>());
                let ex = Op::Extract { archive: tar, dest: dest.join("t"), paths: vec![], password: None, overwrite: Overwrite::Replace };
                assert_eq!(finish(ex), Outcome::Ok);
                dest.join("t")
            } else {
                dest.clone()
            };
            assert_eq!(std::fs::read_to_string(root.join("src/a.txt")).unwrap(), "hello", "{ext}");
            assert_eq!(std::fs::read_to_string(root.join("src/sub dir/é ü.txt")).unwrap(), "unicode", "{ext}");
            let _ = std::fs::remove_dir_all(&dir);
        }
        let _ = parse::parse_listing("");
    }

    #[test]
    fn update_puts_a_file_back_at_its_path() {
        let Some(bin) = sevenzip::binary() else { return };
        let dir = tmp("upd");
        std::fs::create_dir_all(dir.join("src/inner")).unwrap();
        std::fs::write(dir.join("src/inner/f.txt"), "old").unwrap();
        let output = dir.join("u.7z");
        let opts = CreateOpts { output: output.clone(), inputs: vec![dir.join("src")], ..create(Format::SevenZ) };
        assert_eq!(finish(Op::Create(opts)), Outcome::Ok);
        let work = dir.join("work");
        std::fs::create_dir_all(work.join("src/inner")).unwrap();
        std::fs::write(work.join("src/inner/f.txt"), "new!").unwrap();
        let up = Op::Update { archive: output.clone(), base: work, rel: vec!["src/inner/f.txt".into()], password: None };
        assert_eq!(finish(up), Outcome::Ok);
        let l = sevenzip::list(bin, &output, None).unwrap();
        let f: Vec<_> = l.entries.iter().filter(|e| !e.is_dir).collect();
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].path.as_str(), f[0].size), ("src/inner/f.txt", 4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn password_round_trip_and_wrong_password() {
        let Some(bin) = sevenzip::binary() else { return };
        let dir = tmp("pw");
        std::fs::write(dir.join("s.txt"), "secret").unwrap();
        let output = dir.join("p.7z");
        let opts = CreateOpts {
            output: output.clone(),
            inputs: vec![dir.join("s.txt")],
            password: Some("pw1".into()),
            encrypt_names: true,
            ..create(Format::SevenZ)
        };
        assert_eq!(finish(Op::Create(opts)), Outcome::Ok);
        assert_eq!(sevenzip::list(bin, &output, None).map(|_| ()), Err(sevenzip::ListError::NeedsPassword));
        assert_eq!(sevenzip::list(bin, &output, Some("nope")).map(|_| ()), Err(sevenzip::ListError::NeedsPassword));
        assert_eq!(sevenzip::list(bin, &output, Some("pw1")).unwrap().entries.len(), 1);
        let bad = Op::Extract { archive: output.clone(), dest: dir.join("x"), paths: vec![], password: Some("nope".into()), overwrite: Overwrite::Replace };
        assert_eq!(finish(bad), Outcome::WrongPassword);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn create(format: Format) -> CreateOpts {
        CreateOpts {
            format,
            level: 5,
            output: "/out/x.7z".into(),
            inputs: vec!["/in/a b".into()],
            password: None,
            encrypt_names: false,
            solid: true,
            split_mb: None,
            threads: None,
        }
    }

    #[test]
    fn parses_progress_lines() {
        assert_eq!(parse_progress(" 42% 3 + dir/file.txt"), Some((42, "dir/file.txt".into())));
        assert_eq!(parse_progress("  7% 12"), Some((7, "12".into())));
        assert_eq!(parse_progress("100%"), Some((100, "".into())));
        assert_eq!(parse_progress("Everything is Ok"), None);
    }

    #[test]
    fn classifies_failures() {
        assert_eq!(classify_failure("ERROR: Wrong password : a.txt"), Outcome::WrongPassword);
        assert_eq!(classify_failure("x\nERROR: No more files\n"), Outcome::Failed("x\nERROR: No more files".into()));
        assert_eq!(summary("x\nERROR: No more files\n"), "No more files");
        assert_eq!(summary("Scanning\nsomething broke"), "something broke");
    }

    #[test]
    fn extract_command() {
        let p = plan(&Op::Extract {
            archive: "/a/x.zip".into(),
            dest: "/d".into(),
            paths: vec!["docs/a.txt".into()],
            password: Some("pw".into()),
            overwrite: Overwrite::Replace,
        });
        assert_eq!(p.stages[0], ["x", "-y", "-aoa", "-o/d", "-bsp1", "-bso0", "-bse1", "-ppw", "--", "/a/x.zip", "docs/a.txt"]);
    }

    #[test]
    fn seven_z_with_password_and_split() {
        let mut o = create(Format::SevenZ);
        o.password = Some("secret".into());
        o.encrypt_names = true;
        o.split_mb = Some(100);
        let p = plan(&Op::Create(o));
        let a = &p.stages[0];
        assert!(a.contains(&"-mhe=on".to_string()) && a.contains(&"-psecret".to_string()) && a.contains(&"-v100m".to_string()));
        assert!(a.contains(&"-mx=5".to_string()) && a.contains(&"-ms=on".to_string()));
        assert!(p.rename.is_empty());
        assert!(a.contains(&"/out/x.7z".to_string()));
    }

    #[test]
    fn plain_zip_writes_a_part_file_then_renames() {
        let mut o = create(Format::Zip);
        o.output = "/out/x.zip".into();
        o.password = Some("pw".into());
        let p = plan(&Op::Create(o));
        assert!(p.stages[0].contains(&"-mem=AES256".to_string()));
        assert!(!p.stages[0].iter().any(|a| a.starts_with("-ms=")));
        assert_eq!(p.rename, vec![(PathBuf::from("/out/.x.zip.part"), PathBuf::from("/out/x.zip"))]);
    }

    #[test]
    fn tar_gz_is_two_stages() {
        let mut o = create(Format::TarGz);
        o.output = "/out/x.tar.gz".into();
        o.password = Some("ignored".into());
        let p = plan(&Op::Create(o));
        assert_eq!(p.stages.len(), 2);
        assert!(p.stages[0].contains(&"-ttar".to_string()));
        assert!(p.stages[1].contains(&"-tgzip".to_string()));
        assert!(!p.stages.iter().flatten().any(|a| a.starts_with("-p")));
        assert_eq!(p.cleanup.len(), 1);
    }
}
