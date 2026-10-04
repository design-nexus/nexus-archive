//! Parsing `7zz l -slt` (technical listing) output into entries and a folder tree.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    /// Path inside the archive, `/`-separated, with no trailing slash.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub packed: Option<u64>,
    pub modified: String,
    pub encrypted: bool,
    pub method: String,
}

impl Entry {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    pub fn parent(&self) -> &str {
        self.path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Info {
    pub kind: String,
    pub physical_size: u64,
    pub method: String,
    pub solid: bool,
    pub headers_encrypted: bool,
    pub comment: String,
}

#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub info: Info,
    pub entries: Vec<Entry>,
}

fn fields(block: &str) -> BTreeMap<&str, &str> {
    block.lines().filter_map(|l| l.split_once(" = ").or_else(|| l.strip_suffix(" =").map(|k| (k, "")))).collect()
}

pub fn parse_listing(text: &str) -> Listing {
    let text = text.replace("\r\n", "\n");
    let (head, body) = match text.split_once("\n----------\n") {
        Some((h, b)) => (h, b),
        None => ("", text.as_str()),
    };
    let h = fields(head);
    let num = |m: &BTreeMap<&str, &str>, k: &str| m.get(k).and_then(|v| v.trim().parse::<u64>().ok());
    let info = Info {
        kind: h.get("Type").unwrap_or(&"").to_string(),
        physical_size: num(&h, "Physical Size").unwrap_or(0),
        method: h.get("Method").unwrap_or(&"").to_string(),
        solid: h.get("Solid") == Some(&"+"),
        headers_encrypted: h.get("Headers Encrypted") == Some(&"+"),
        comment: h.get("Comment").unwrap_or(&"").to_string(),
    };
    let mut entries = Vec::new();
    for block in body.split("\n\n") {
        let f = fields(block);
        // Single-stream formats (gz, xz, bz2) list one file with no name: it is the
        // archive's own name without its last extension.
        let own = h.get("Path").map(|p| {
            let name = p.rsplit('/').next().unwrap_or(p);
            name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name).to_string()
        });
        let path = match f.get("Path") {
            Some(p) => p.to_string(),
            None if f.contains_key("Size") => own.unwrap_or_default(),
            None => continue,
        };
        let path = path.replace('\\', "/");
        let path = path.trim_matches('/').to_string();
        if path.is_empty() {
            continue;
        }
        let is_dir = f.get("Folder") == Some(&"+") || f.get("Attributes").is_some_and(|a| a.starts_with('D'));
        entries.push(Entry {
            path,
            is_dir,
            size: num(&f, "Size").unwrap_or(0),
            packed: num(&f, "Packed Size"),
            modified: f.get("Modified").map(|m| m.split('.').next().unwrap_or(m).trim().to_string()).unwrap_or_default(),
            encrypted: f.get("Encrypted") == Some(&"+"),
            method: f.get("Method").unwrap_or(&"").to_string(),
        });
    }
    Listing { info, entries }
}

impl Listing {
    /// A gzip/xz/bzip2 stream that holds one tar: the real contents are one level down.
    pub fn is_tar_wrapper(&self) -> bool {
        self.entries.len() == 1 && !self.entries[0].is_dir && self.entries[0].path.to_lowercase().ends_with(".tar") && self.info.kind != "tar"
    }

    pub fn total_size(&self) -> u64 {
        self.entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum()
    }

    pub fn file_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_dir).count()
    }

    pub fn encrypted(&self) -> bool {
        self.info.headers_encrypted || self.entries.iter().any(|e| e.encrypted)
    }

    /// The direct children of `dir` ("" is the root). Folders that only exist
    /// implicitly (a file at `a/b/c.txt` with no `a` entry) are synthesised, with
    /// their size summed from the files below.
    pub fn children(&self, dir: &str) -> Vec<Entry> {
        let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
        let mut out: BTreeMap<String, Entry> = BTreeMap::new();
        for e in &self.entries {
            let Some(rest) = e.path.strip_prefix(&prefix) else { continue };
            if rest.is_empty() {
                continue;
            }
            let (first, deeper) = match rest.split_once('/') {
                Some((f, _)) => (f, true),
                None => (rest, false),
            };
            let path = format!("{prefix}{first}");
            let slot = out.entry(path.clone()).or_insert_with(|| Entry { path, is_dir: deeper, ..Default::default() });
            if deeper {
                slot.is_dir = true;
                if !e.is_dir {
                    slot.size += e.size;
                    slot.packed = Some(slot.packed.unwrap_or(0) + e.packed.unwrap_or(0));
                }
            } else {
                let size_so_far = slot.size;
                let packed_so_far = slot.packed;
                *slot = e.clone();
                if e.is_dir {
                    slot.size += size_so_far;
                    slot.packed = packed_so_far.or(slot.packed);
                }
            }
        }
        out.into_values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Path = /tmp/a.7z
Type = 7z
Physical Size = 1234
Headers Size = 150
Method = LZMA2:24
Solid = -
Blocks = 2

----------
Path = docs
Folder = +
Size = 0
Packed Size = 0
Modified = 2024-05-01 10:00:00.123456789
Attributes = D drwxr-xr-x
Encrypted = -

Path = docs/read me.txt
Folder = -
Size = 500
Packed Size = 200
Modified = 2024-05-01 10:00:01.000000000
Attributes = A -rw-r--r--
Encrypted = +
Method = LZMA2:24

Path = top.txt
Folder = -
Size = 20
Packed Size = 20
Modified = 2024-05-02 08:00:00.000000000
Encrypted = -

Path = deep/er/file é.bin
Folder = -
Size = 7
Packed Size = 7
";

    #[test]
    fn parses_info_and_entries() {
        let l = parse_listing(SAMPLE);
        assert_eq!(l.info.kind, "7z");
        assert_eq!(l.info.physical_size, 1234);
        assert!(!l.info.solid);
        assert_eq!(l.entries.len(), 4);
        assert!(l.entries[0].is_dir);
        assert_eq!(l.entries[1].name(), "read me.txt");
        assert_eq!(l.entries[1].parent(), "docs");
        assert!(l.entries[1].encrypted);
        assert_eq!(l.entries[1].modified, "2024-05-01 10:00:01");
        assert!(l.encrypted());
        assert_eq!(l.file_count(), 3);
        assert_eq!(l.total_size(), 527);
    }

    #[test]
    fn lists_children_with_implicit_folders() {
        let l = parse_listing(SAMPLE);
        let root: Vec<_> = l.children("").iter().map(|e| (e.path.clone(), e.is_dir, e.size)).collect();
        assert_eq!(
            root,
            vec![("deep".into(), true, 7), ("docs".into(), true, 500), ("top.txt".into(), false, 20)]
        );
        let deep = l.children("deep");
        assert_eq!(deep.len(), 1);
        assert_eq!(deep[0].path, "deep/er");
        assert_eq!(l.children("deep/er")[0].name(), "file é.bin");
    }

    #[test]
    fn nameless_single_stream_entry_takes_the_archive_name() {
        let l = parse_listing("Path = /x/files.tar.xz\nType = xz\nPhysical Size = 56\n\n----------\nSize = 3\nPacked Size = 56\nMethod = LZMA2:12\n");
        assert_eq!(l.entries.len(), 1);
        assert_eq!(l.entries[0].path, "files.tar");
        assert!(l.is_tar_wrapper());
    }

    #[test]
    fn empty_output_is_empty() {
        assert!(parse_listing("").entries.is_empty());
    }
}
