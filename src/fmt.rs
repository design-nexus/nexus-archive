//! Small formatting helpers.

/// "1 page", "12 pages".
pub fn count(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", thousands(n), if n == 1 { one } else { many })
}

/// 1,204
pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 480 KB, 4.2 MB, 1.1 GB
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", (b / 1e3).max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_counts() {
        assert_eq!(count(1, "page", "pages"), "1 page");
        assert_eq!(count(1204, "page", "pages"), "1,204 pages");
        assert_eq!(thousands(123), "123");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(size(4_200_000), "4.2 MB");
        assert_eq!(size(300), "1 KB");
        assert_eq!(size(2_500_000_000), "2.5 GB");
    }
}
