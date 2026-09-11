//! Compare and diff logic.

use anyhow::Result;
use std::collections::HashMap;

use crate::record::{FileRecord, FileType};

/// Streaming compare: merges live/current records against a baseline in DFS order.
/// Both sides must be in the same DFS walk order (pre-order, sorted siblings).
pub struct MergeCompare {
    baseline: Vec<FileRecord>,
    cursor: usize,
}

impl MergeCompare {
    /// Create from a baseline in DFS order.
    pub fn new(baseline: Vec<FileRecord>) -> Self {
        MergeCompare {
            baseline,
            cursor: 0,
        }
    }

    /// Process one live/current record. Returns diff lines for any missing
    /// baseline records (emitted first) plus a change/leftover line if applicable.
    pub fn process(&mut self, record: &FileRecord, verbose: bool) -> Vec<String> {
        let mut lines = Vec::new();
        // Advance cursor: emit baseline records that come before this live record
        // in DFS order (i.e., they are in a subtree the live walk has left).
        while self.cursor < self.baseline.len()
            && crate::format::dfs_after(&self.baseline[self.cursor].path, &record.path)
        {
            let missing_path = &self.baseline[self.cursor].path;
            lines.push(format!("-{}  {}", ".............", missing_path));
            self.cursor += 1;
        }
        // Check for match or leftover
        if self.cursor < self.baseline.len() && self.baseline[self.cursor].path == record.path {
            let base = &self.baseline[self.cursor];
            if let Some(line) = change_line(base, record, verbose) {
                lines.push(line);
            }
            self.cursor += 1;
        } else {
            // Not in baseline: leftover (new file)
            lines.push(format!("!{}  {}", ".............", record.path));
        }
        lines
    }

    /// After the stream is exhausted, emit remaining missing files.
    pub fn finish(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        while self.cursor < self.baseline.len() {
            lines.push(format!(
                "-{}  {}",
                ".............", self.baseline[self.cursor].path
            ));
            self.cursor += 1;
        }
        lines
    }
}

/// One diff line for a record that appears in both baseline and current,
/// or None if unchanged.
fn change_line(base: &FileRecord, curr: &FileRecord, verbose: bool) -> Option<String> {
    let flags = compare_flags(base, curr);
    if flags.chars().all(|c| c == '.') {
        return None;
    }
    let mut line = format!(" {}  {}", flags, curr.path);
    if verbose {
        line.push_str(&format_detail(base, curr));
    }
    Some(line)
}

/// Compare two records and return the 13-character flag string.
fn compare_flags(base: &FileRecord, curr: &FileRecord) -> String {
    let mut flags = ['.'; 13];
    compare_flags_basic(base, curr, &mut flags);
    compare_flags_extended(base, curr, &mut flags);
    flags.iter().collect()
}

fn compare_flags_basic(base: &FileRecord, curr: &FileRecord, flags: &mut [char; 13]) {
    // 0: s — size differs
    if base.file_type == FileType::Regular
        && curr.file_type == FileType::Regular
        && base.size != curr.size
    {
        flags[0] = 's';
    }
    // 1: M — mode differs
    if base.mode != curr.mode {
        flags[1] = 'M';
    }
    // 2: D — device numbers differ
    if base.file_type == FileType::CharDev || base.file_type == FileType::BlockDev {
        let d1 = (base.dev_major.unwrap_or(0), base.dev_minor.unwrap_or(0));
        let d2 = (curr.dev_major.unwrap_or(0), curr.dev_minor.unwrap_or(0));
        if d1 != d2 {
            flags[2] = 'D';
        }
    }
    // 3: G — group differs
    if base.group != curr.group {
        flags[3] = 'G';
    }
    // 4: U — user/owner differs
    if base.user != curr.user {
        flags[4] = 'U';
    }
    // 5: P — permissions differ (special bits: setuid, setgid, sticky)
    if base.perms != curr.perms {
        flags[5] = 'P';
    }
    // 6: L — hard link count differs
    if base.hardlinks != curr.hardlinks {
        flags[6] = 'L';
    }
}

fn compare_flags_extended(base: &FileRecord, curr: &FileRecord, flags: &mut [char; 13]) {
    // 8: S — symlink target differs
    if base.file_type == FileType::Symlink
        && curr.file_type == FileType::Symlink
        && base.symlink_target != curr.symlink_target
    {
        flags[8] = 'S';
    }
    // 9: C/? — content differs
    if base.file_type == FileType::Regular && curr.file_type == FileType::Regular {
        if base.checksum_skipped || curr.checksum_skipped {
            flags[9] = '?';
        } else if base.checksum != curr.checksum {
            flags[9] = 'C';
        }
    }
    // 10: X — xattrs differ
    if xattr_map(base) != xattr_map(curr) {
        flags[10] = 'X';
    }
    // 11: A — file attributes differ (lsattr)
    if base.file_attrs != curr.file_attrs {
        flags[11] = 'A';
    }
    // 12: F — file type changed
    if base.file_type != curr.file_type {
        flags[12] = 'F';
    }
}

fn format_detail(base: &FileRecord, curr: &FileRecord) -> String {
    let mut details = Vec::new();
    format_detail_basic(base, curr, &mut details);
    format_detail_extended(base, curr, &mut details);
    format!(" ({})", details.join(" "))
}

fn format_detail_basic(base: &FileRecord, curr: &FileRecord, details: &mut Vec<String>) {
    // s: size
    if base.size != curr.size {
        details.push(format!(
            "s:{}→{}",
            base.size.unwrap_or(0),
            curr.size.unwrap_or(0)
        ));
    }
    // M: mode
    if base.mode != curr.mode {
        details.push(format!("M:{:o}→{:o}", base.mode, curr.mode));
    }
    // D: device
    if (base.file_type == FileType::CharDev || base.file_type == FileType::BlockDev)
        && (base.dev_major, base.dev_minor) != (curr.dev_major, curr.dev_minor)
    {
        details.push(format!(
            "D:{}:{}→{}:{}",
            base.dev_major.unwrap_or(0),
            base.dev_minor.unwrap_or(0),
            curr.dev_major.unwrap_or(0),
            curr.dev_minor.unwrap_or(0)
        ));
    }
    // G: group
    if base.group != curr.group {
        details.push(format!("G:{}→{}", base.group, curr.group));
    }
    // U: user
    if base.user != curr.user {
        details.push(format!("U:{}→{}", base.user, curr.user));
    }
    // P: perms
    if base.perms != curr.perms {
        details.push(format!("P:{:b}→{:b}", base.perms, curr.perms));
    }
    // L: hardlinks
    if base.hardlinks != curr.hardlinks {
        details.push(format!("L:{}→{}", base.hardlinks, curr.hardlinks));
    }
}

fn format_detail_extended(base: &FileRecord, curr: &FileRecord, details: &mut Vec<String>) {
    push_symlink_diff(base, curr, details);
    push_checksum_diff(base, curr, details);
    push_xattrs_diff(base, curr, details);
    push_file_attrs_diff(base, curr, details);
    push_file_type_diff(base, curr, details);
}

// S: symlink target
fn push_symlink_diff(base: &FileRecord, curr: &FileRecord, details: &mut Vec<String>) {
    let either_symlink = base.file_type == FileType::Symlink || curr.file_type == FileType::Symlink;
    if either_symlink && base.symlink_target != curr.symlink_target {
        details.push(format!(
            "S:{}→{}",
            base.symlink_target.as_deref().unwrap_or("<none>"),
            curr.symlink_target.as_deref().unwrap_or("<none>")
        ));
    }
}

// C: checksum (regular files only)
fn push_checksum_diff(base: &FileRecord, curr: &FileRecord, details: &mut Vec<String>) {
    if base.file_type != FileType::Regular || curr.file_type != FileType::Regular {
        return;
    }
    if base.checksum_skipped || curr.checksum_skipped {
        details.push("C:? (checksum skipped)".to_string());
    } else if base.checksum != curr.checksum {
        details.push(format!(
            "C:{}→{}",
            short_hex(&base.checksum),
            short_hex(&curr.checksum)
        ));
    }
}

// X: xattrs
fn push_xattrs_diff(base: &FileRecord, curr: &FileRecord, details: &mut Vec<String>) {
    if xattr_map(base) != xattr_map(curr) {
        details.push("X:xattrs differ".to_string());
    }
}

// A: file attrs
fn push_file_attrs_diff(base: &FileRecord, curr: &FileRecord, details: &mut Vec<String>) {
    if base.file_attrs != curr.file_attrs {
        details.push(format!("A:{}→{}", base.file_attrs, curr.file_attrs));
    }
}

// F: file type
fn push_file_type_diff(base: &FileRecord, curr: &FileRecord, details: &mut Vec<String>) {
    if base.file_type != curr.file_type {
        details.push(format!("F:{:?}→{:?}", base.file_type, curr.file_type));
    }
}

/// First 7 hex chars of a checksum (or the whole thing if shorter).
fn short_hex(checksum: &Option<Vec<u8>>) -> String {
    let hex = hex(checksum);
    hex.get(..7).map(String::from).unwrap_or(hex)
}

fn hex(checksum: &Option<Vec<u8>>) -> String {
    checksum
        .as_deref()
        .map(|c| c.iter().map(|b| format!("{:02x}", b)).collect::<String>())
        .unwrap_or_default()
}

fn xattr_map(r: &FileRecord) -> HashMap<&str, &Vec<u8>> {
    r.xattrs.iter().map(|(k, v)| (k.as_str(), v)).collect()
}

/// Diff two snapshot files.
pub fn diff_snapshots(path_a: &str, path_b: &str, verbose: bool) -> Result<Vec<String>> {
    let baseline = crate::format::read_snapshot(path_a)?;
    let current = crate::format::read_snapshot(path_b)?;
    let mut mc = MergeCompare::new(baseline);
    let mut lines: Vec<String> = Vec::new();
    for record in &current {
        lines.extend(mc.process(record, verbose));
    }
    lines.extend(mc.finish());
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::FileRecord;

    /// Minimal regular file record with just path and size.
    fn reg(path: &str, size: u64) -> FileRecord {
        FileRecord {
            path: path.to_string(),
            file_type: FileType::Regular,
            size: Some(size),
            ..Default::default()
        }
    }

    fn regular(path: &str, size: u64, mode: u32, checksum: &[u8]) -> FileRecord {
        FileRecord {
            path: path.to_string(),
            file_type: FileType::Regular,
            mode,
            size: Some(size),
            checksum: Some(checksum.to_vec()),
            ..Default::default()
        }
    }

    fn flag_at(flags: &str, pos: usize) -> char {
        flags.chars().nth(pos).unwrap()
    }

    /// Extract flag string from a change line like " sM......D...  /path".
    fn extract_flags(line: &str) -> String {
        line[1..14].chars().take(13).collect()
    }

    #[test]
    fn merge_detects_leftover_and_changed() {
        // Flat structure: all at same depth
        let baseline = vec![reg("/a", 100), reg("/b", 200), reg("/c", 300)];
        let mut mc = MergeCompare::new(baseline);

        assert!(mc.process(&reg("/a", 100), false).is_empty());

        let lines = mc.process(&reg("/b", 999), false);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with(" s"), "got: {}", lines[0]);

        let lines = mc.process(&reg("/d", 400), false);
        // /c is missing, /d is leftover
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with('-') && lines[0].contains("/c"));
        assert!(lines[1].starts_with('!') && lines[1].contains("/d"));
        assert!(mc.finish().is_empty());
    }

    #[test]
    fn merge_no_changes_no_missing() {
        let baseline = vec![reg("/a", 100)];
        let mut mc = MergeCompare::new(baseline);

        assert!(mc.process(&reg("/a", 100), false).is_empty());
        assert!(mc.finish().is_empty());
    }

    #[test]
    fn merge_empty_baseline_all_leftovers() {
        let baseline: Vec<FileRecord> = Vec::new();
        let mut mc = MergeCompare::new(baseline);

        let lines = mc.process(&reg("/a", 100), false);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with('!'));
        assert!(mc.finish().is_empty());
    }

    #[test]
    fn merge_all_missing_emitted_in_finish() {
        let baseline = vec![reg("/a", 2), reg("/m", 3), reg("/z", 1)];
        let mut mc = MergeCompare::new(baseline);

        let missing = mc.finish();
        assert_eq!(missing.len(), 3);
        assert!(missing[0].contains("/a"));
        assert!(missing[1].contains("/m"));
        assert!(missing[2].contains("/z"));
    }

    #[test]
    fn merge_multiple_changes_on_one_file() {
        let mut base = reg("/f", 100);
        base.user = "root".into();
        base.group = "wheel".into();
        base.mode = 0o644;
        let baseline = vec![base];
        let mut mc = MergeCompare::new(baseline);

        let mut live = reg("/f", 200);
        live.user = "www".into();
        live.group = "www".into();
        live.mode = 0o755;

        let lines = mc.process(&live, false);
        assert_eq!(lines.len(), 1);
        let flags = extract_flags(&lines[0]);
        assert!(flags.contains('s'));
        assert!(flags.contains('M'));
        assert!(flags.contains('U'));
        assert!(flags.contains('G'));
    }

    #[test]
    fn merge_type_change_detected() {
        let baseline = vec![reg("/f", 100)];
        let mut mc = MergeCompare::new(baseline);

        let live = FileRecord {
            path: "/f".into(),
            file_type: FileType::Symlink,
            symlink_target: Some("/target".into()),
            ..Default::default()
        };

        let lines = mc.process(&live, false);
        assert_eq!(lines.len(), 1);
        let flags = extract_flags(&lines[0]);
        assert!(flags.ends_with('F'));
    }

    #[test]
    fn merge_verbose_output_includes_details() {
        let baseline = vec![reg("/f", 100)];
        let mut mc = MergeCompare::new(baseline);

        let lines = mc.process(&reg("/f", 200), true);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("s:100\u{2192}200"));
    }

    #[test]
    fn merge_directory_tree_structure() {
        // DFS order: /home/user, /home/user/a.txt, /home/user/b.txt, /home/user/c.txt
        let baseline = vec![
            FileRecord {
                path: "/home/user".into(),
                file_type: FileType::Directory,
                ..Default::default()
            },
            reg("/home/user/a.txt", 1),
            reg("/home/user/b.txt", 2),
            reg("/home/user/c.txt", 3),
        ];
        let mut mc = MergeCompare::new(baseline);

        // Directory matches
        let dir_record = FileRecord {
            path: "/home/user".into(),
            file_type: FileType::Directory,
            ..Default::default()
        };
        assert!(mc.process(&dir_record, false).is_empty());

        assert!(mc.process(&reg("/home/user/a.txt", 1), false).is_empty());

        // b.txt is missing, c.txt changed
        let lines = mc.process(&reg("/home/user/c.txt", 99), false);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("/home/user/b.txt") && lines[0].starts_with('-'));
        assert!(lines[1].contains("/home/user/c.txt"));

        // d.txt is leftover
        let lines = mc.process(&reg("/home/user/d.txt", 4), false);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with('!'));
        assert!(mc.finish().is_empty());
    }

    /// Test the prefix-collision case: journal/ vs journal.conf
    #[test]
    fn merge_dfs_order_prefix_collision() {
        // DFS order: journal (dir), journal/profile (file), journal.conf (file)
        let baseline = vec![
            FileRecord {
                path: "/etc/journal".into(),
                file_type: FileType::Directory,
                ..Default::default()
            },
            reg("/etc/journal/profile", 10),
            reg("/etc/journal.conf", 20),
        ];
        let mut mc = MergeCompare::new(baseline);

        // All match — no diffs
        let dir = FileRecord {
            path: "/etc/journal".into(),
            file_type: FileType::Directory,
            ..Default::default()
        };
        assert!(mc.process(&dir, false).is_empty());

        assert!(mc
            .process(&reg("/etc/journal/profile", 10), false)
            .is_empty());
        assert!(mc.process(&reg("/etc/journal.conf", 20), false).is_empty());
        assert!(mc.finish().is_empty());
    }

    /// Test that a missing file inside a directory is detected.
    #[test]
    fn merge_dfs_missing_in_subdirectory() {
        // DFS order: journal (dir), journal/profile (file), journal.conf (file)
        // Live: journal (dir), journal.conf  (profile missing)
        let baseline = vec![
            FileRecord {
                path: "/etc/journal".into(),
                file_type: FileType::Directory,
                ..Default::default()
            },
            reg("/etc/journal/profile", 10),
            reg("/etc/journal.conf", 20),
        ];
        let mut mc = MergeCompare::new(baseline);

        let dir = FileRecord {
            path: "/etc/journal".into(),
            file_type: FileType::Directory,
            ..Default::default()
        };
        assert!(mc.process(&dir, false).is_empty());

        // journal.conf: should emit journal/profile as missing, then match journal.conf
        let lines = mc.process(&reg("/etc/journal.conf", 20), false);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with('-') && lines[0].contains("/profile"));
        assert!(mc.finish().is_empty());
    }

    #[test]
    fn flags_all_unchanged() {
        let a = regular("/f", 100, 0o644, b"abc");
        let b = regular("/f", 100, 0o644, b"abc");
        let flags = compare_flags(&a, &b);
        assert_eq!(flags, ".............");
    }

    #[test]
    fn flags_size_change() {
        let a = regular("/f", 100, 0o644, b"abc");
        let b = regular("/f", 200, 0o644, b"def");
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 0), 's');
    }

    #[test]
    fn flags_mode_change() {
        let a = regular("/f", 100, 0o644, b"abc");
        let b = regular("/f", 100, 0o755, b"abc");
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 1), 'M');
    }

    #[test]
    fn flags_group_change() {
        let mut a = regular("/f", 100, 0o644, b"abc");
        a.group = "wheel".into();
        let mut b = regular("/f", 100, 0o644, b"abc");
        b.group = "staff".into();
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 3), 'G');
    }

    #[test]
    fn flags_user_change() {
        let mut a = regular("/f", 100, 0o644, b"abc");
        a.user = "root".into();
        let mut b = regular("/f", 100, 0o644, b"abc");
        b.user = "www".into();
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 4), 'U');
    }

    #[test]
    fn flags_perms_change() {
        let mut a = regular("/f", 100, 0o644, b"abc");
        a.perms = 0; // no special bits
        let mut b = regular("/f", 100, 0o644, b"abc");
        b.perms = 1; // setuid
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 5), 'P');
    }

    #[test]
    fn flags_hardlink_change() {
        let mut a = regular("/f", 100, 0o644, b"abc");
        a.hardlinks = 1;
        let mut b = regular("/f", 100, 0o644, b"abc");
        b.hardlinks = 2;
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 6), 'L');
    }

    #[test]
    fn flags_symlink_target_change() {
        let mut a = FileRecord {
            path: "/l".into(),
            ..Default::default()
        };
        a.file_type = FileType::Symlink;
        a.symlink_target = Some("/a".into());
        let mut b = FileRecord {
            path: "/l".into(),
            ..Default::default()
        };
        b.file_type = FileType::Symlink;
        b.symlink_target = Some("/b".into());
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 8), 'S');
    }

    #[test]
    fn flags_checksum_mismatch() {
        let a = regular("/f", 100, 0o644, b"abc");
        let b = regular("/f", 100, 0o644, b"xyz");
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 9), 'C');
    }

    #[test]
    fn flags_checksum_skipped() {
        let mut a = regular("/f", 100, 0o644, b"");
        a.checksum_skipped = true;
        let b = regular("/f", 100, 0o644, b"abc");
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 9), '?');
    }

    #[test]
    fn flags_xattr_change() {
        let mut a = regular("/f", 100, 0o644, b"abc");
        a.xattrs.push(("user.test".into(), b"1".to_vec()));
        let mut b = regular("/f", 100, 0o644, b"abc");
        b.xattrs.push(("user.test".into(), b"2".to_vec()));
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 10), 'X');
    }

    #[test]
    fn flags_file_attrs_change() {
        let mut a = regular("/f", 100, 0o644, b"abc");
        a.file_attrs = "i".into();
        let b = regular("/f", 100, 0o644, b"abc");
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 11), 'A');
    }

    #[test]
    fn flags_type_change() {
        let a = regular("/f", 100, 0o644, b"abc");
        let mut b = regular("/f", 100, 0o644, b"abc");
        b.file_type = FileType::Symlink;
        b.symlink_target = Some("/x".into());
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 12), 'F');
    }

    #[test]
    fn compare_missing_file() {
        let baseline = vec![regular("/f", 100, 0o644, b"abc")];
        let mut mc = MergeCompare::new(baseline);
        let missing = mc.finish();
        assert_eq!(missing.len(), 1);
        assert!(
            missing[0].starts_with('-'),
            "expected missing prefix, got: {}",
            missing[0]
        );
    }

    #[test]
    fn compare_leftover_file() {
        let mut mc = MergeCompare::new(Vec::new());
        let lines = mc.process(&regular("/f", 100, 0o644, b"abc"), false);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].starts_with('!'),
            "expected leftover prefix, got: {}",
            lines[0]
        );
    }

    #[test]
    fn compare_unchanged_not_reported() {
        let a = regular("/f", 100, 0o644, b"abc");
        let mut mc = MergeCompare::new(vec![a.clone()]);
        assert!(mc.process(&a, false).is_empty());
        assert!(mc.finish().is_empty());
    }

    #[test]
    fn compare_multiple_changes_all_flagged() {
        let mut a = regular("/f", 100, 0o644, b"abc");
        a.user = "root".into();
        a.group = "wheel".into();
        let mut b = regular("/f", 200, 0o755, b"xyz");
        b.user = "www".into();
        b.group = "www".into();
        let flags = compare_flags(&a, &b);
        assert_eq!(flag_at(&flags, 0), 's'); // size
        assert_eq!(flag_at(&flags, 1), 'M'); // mode
        assert_eq!(flag_at(&flags, 3), 'G'); // group
        assert_eq!(flag_at(&flags, 4), 'U'); // user
        assert_eq!(flag_at(&flags, 9), 'C'); // content
    }

    #[test]
    fn verbose_detail_includes_field_names() {
        let a = regular("/f", 100, 0o644, b"abc");
        let b = regular("/f", 200, 0o755, b"xyz");
        let detail = format_detail(&a, &b);
        assert!(detail.contains("s:"), "should include size detail");
        assert!(detail.contains("M:"), "should include mode detail");
        assert!(detail.contains("C:"), "should include content detail");
    }
}
