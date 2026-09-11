//! Binary snapshot format: read and write.
//!
//! v1: version + records (streamed) + trailer (file count for verification).
//!
//! The format is designed for streaming: the writer does NOT need to know
//! the file count upfront. The trailer at the end provides post-hoc
//! verification that all records were written.

use anyhow::{bail, Context, Result};
use std::io::{BufReader, Read, Write};

use crate::record::{FileRecord, FileType};

const TAG_END: u16 = 0xFFFF;
const TAG_PATH: u16 = 0x0001;
const TAG_TYPE: u16 = 0x0002;
const TAG_MODE: u16 = 0x0003;
const TAG_USER: u16 = 0x0004;
const TAG_GROUP: u16 = 0x0005;
const TAG_PERMS: u16 = 0x0006;
const TAG_HARDLINKS: u16 = 0x0007;
const TAG_SIZE: u16 = 0x0008;
const TAG_MTIME: u16 = 0x0009;
const TAG_CHECKSUM: u16 = 0x000A;
// 0x000B reserved (magic_type, not yet captured)
const TAG_SYMLINK: u16 = 0x000C;
const TAG_DEV_MAJOR: u16 = 0x000D;
const TAG_DEV_MINOR: u16 = 0x000E;
const TAG_XATTR: u16 = 0x000F;
const TAG_FILE_ATTRS: u16 = 0x0010;
const TAG_CHECKSUM_SKIPPED: u16 = 0x0011;

// Trailer magic: distinguishes the trailer from a record tag.
// Records start with tag 0x0001..0x0011 or 0xFFFF (end-of-record).
// The trailer uses 0xFE00 to be unambiguous.
const TRAILER_MAGIC: u16 = 0xFE00;

const VERSION: u16 = 1;

/// Write the snapshot header (just the version).
pub fn write_header<W: Write>(w: &mut W) -> Result<()> {
    w.write_all(&VERSION.to_le_bytes())
        .context("write version")?;
    Ok(())
}

/// Write the trailer (file count for verification).
pub fn write_trailer<W: Write>(file_count: u64, w: &mut W) -> Result<()> {
    w.write_all(&TRAILER_MAGIC.to_le_bytes())
        .context("write trailer magic")?;
    w.write_all(&file_count.to_le_bytes())
        .context("write trailer count")?;
    Ok(())
}

/// Write a single record.
pub fn write_record<W: Write>(r: &FileRecord, w: &mut W) -> Result<()> {
    write_record_fixed(r, w)?;
    write_record_optionals(r, w)?;
    write_record_xattrs(r, w)?;
    write_field(TAG_FILE_ATTRS, r.file_attrs.as_bytes(), w)?;

    // End-of-record sentinel
    w.write_all(&TAG_END.to_le_bytes())
        .context("write end-of-record")?;

    Ok(())
}

fn write_record_fixed<W: Write>(r: &FileRecord, w: &mut W) -> Result<()> {
    write_field(TAG_PATH, r.path.as_bytes(), w)?;
    write_field(TAG_TYPE, &[r.file_type.to_u8()], w)?;
    write_field(TAG_MODE, &r.mode.to_le_bytes(), w)?;
    write_field(TAG_USER, r.user.as_bytes(), w)?;
    write_field(TAG_GROUP, r.group.as_bytes(), w)?;
    write_field(TAG_PERMS, &[r.perms], w)?;
    write_field(TAG_HARDLINKS, &r.hardlinks.to_le_bytes(), w)?;
    Ok(())
}

fn write_record_optionals<W: Write>(r: &FileRecord, w: &mut W) -> Result<()> {
    if let Some(size) = r.size {
        write_field(TAG_SIZE, &size.to_le_bytes(), w)?;
    }
    if let Some(mtime) = r.mtime {
        write_field(TAG_MTIME, &mtime.to_le_bytes(), w)?;
    }
    if let Some(checksum) = &r.checksum {
        write_field(TAG_CHECKSUM, checksum, w)?;
    }
    if r.checksum_skipped {
        write_field(TAG_CHECKSUM_SKIPPED, b"", w)?;
    }
    if let Some(target) = &r.symlink_target {
        write_field(TAG_SYMLINK, target.as_bytes(), w)?;
    }
    if let Some(major) = r.dev_major {
        write_field(TAG_DEV_MAJOR, &major.to_le_bytes(), w)?;
    }
    if let Some(minor) = r.dev_minor {
        write_field(TAG_DEV_MINOR, &minor.to_le_bytes(), w)?;
    }
    Ok(())
}

fn write_record_xattrs<W: Write>(r: &FileRecord, w: &mut W) -> Result<()> {
    for (key, value) in &r.xattrs {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(key.len() as u16).to_le_bytes());
        buf.extend_from_slice(key.as_bytes());
        buf.extend_from_slice(value);
        write_field(TAG_XATTR, &buf, w)?;
    }
    Ok(())
}

fn write_field<W: Write>(tag: u16, value: &[u8], w: &mut W) -> Result<()> {
    w.write_all(&tag.to_le_bytes()).context("write tag")?;
    w.write_all(&(value.len() as u32).to_le_bytes())
        .context("write length")?;
    w.write_all(value).context("write value")?;
    Ok(())
}

/// One unit read from the snapshot stream: a record or the trailer.
enum ReadItem {
    Record(Box<FileRecord>),
    Trailer(u64),
}

/// Read one item from the stream. Returns None at EOF.
fn read_record<R: Read>(r: &mut R) -> Result<Option<ReadItem>> {
    let mut tag_buf = [0u8; 2];
    match r.read_exact(&mut tag_buf) {
        Ok(()) => {}
        Err(_) => return Ok(None), // EOF
    }
    let tag = u16::from_le_bytes(tag_buf);

    if tag == TAG_END {
        return Ok(None);
    }
    if tag == TRAILER_MAGIC {
        let mut count_buf = [0u8; 8];
        r.read_exact(&mut count_buf).context("read trailer count")?;
        let file_count = u64::from_le_bytes(count_buf);
        return Ok(Some(ReadItem::Trailer(file_count)));
    }

    let mut record = FileRecord::default();

    // First field (tag already read)
    let (_, value) = read_value(r)?;
    parse_field(&mut record, tag, &value)?;

    // Subsequent fields
    loop {
        let mut t = [0u8; 2];
        r.read_exact(&mut t).context("read tag")?;
        let tag = u16::from_le_bytes(t);
        if tag == TAG_END {
            break;
        }
        if tag == TRAILER_MAGIC {
            // Trailer encountered mid-record — malformed
            bail!("trailer magic found inside record");
        }
        let (_, value) = read_value(r)?;
        parse_field(&mut record, tag, &value)?;
    }

    Ok(Some(ReadItem::Record(Box::new(record))))
}

/// Read a u32 length prefix and the value bytes that follow it.
fn read_value<R: Read>(r: &mut R) -> Result<(u32, Vec<u8>)> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).context("read length")?;
    let len = u32::from_le_bytes(len_buf);
    let mut value = vec![0u8; len as usize];
    r.read_exact(&mut value).context("read value")?;
    Ok((len, value))
}

fn parse_field(r: &mut FileRecord, tag: u16, value: &[u8]) -> Result<()> {
    match tag {
        TAG_PATH => r.path = parse_string(value, "path")?,
        TAG_TYPE => r.file_type = FileType::from(value[0]),
        TAG_MODE => r.mode = u32::from_le_bytes(value.try_into().unwrap()),
        TAG_USER => r.user = parse_string(value, "user")?,
        TAG_GROUP => r.group = parse_string(value, "group")?,
        TAG_PERMS => r.perms = value[0],
        TAG_HARDLINKS => r.hardlinks = u32::from_le_bytes(value.try_into().unwrap()),
        TAG_SIZE => r.size = Some(u64::from_le_bytes(value.try_into().unwrap())),
        TAG_MTIME => r.mtime = Some(i64::from_le_bytes(value.try_into().unwrap())),
        TAG_CHECKSUM => r.checksum = Some(value.to_vec()),
        TAG_SYMLINK => r.symlink_target = Some(parse_string(value, "symlink_target")?),
        TAG_DEV_MAJOR => r.dev_major = Some(u32::from_le_bytes(value.try_into().unwrap())),
        TAG_DEV_MINOR => r.dev_minor = Some(u32::from_le_bytes(value.try_into().unwrap())),
        TAG_XATTR => parse_xattr(r, value)?,
        TAG_FILE_ATTRS => r.file_attrs = parse_string(value, "file_attrs")?,
        TAG_CHECKSUM_SKIPPED => r.checksum_skipped = true,
        _ => {
            // Unknown tag: skip (forward compatibility)
        }
    }
    Ok(())
}

fn parse_string(value: &[u8], field: &str) -> Result<String> {
    String::from_utf8(value.to_vec()).with_context(|| format!("{field} is not valid UTF-8"))
}

fn parse_xattr(r: &mut FileRecord, value: &[u8]) -> Result<()> {
    if value.len() < 2 {
        bail!("xattr value too short");
    }
    let key_len = u16::from_le_bytes([value[0], value[1]]) as usize;
    if value.len() < 2 + key_len {
        bail!("xattr key extends beyond value");
    }
    let key = parse_string(&value[2..2 + key_len], "xattr key")?;
    let val = value[2 + key_len..].to_vec();
    r.xattrs.push((key, val));
    Ok(())
}

/// Returns true if path `b` comes after path `a` in DFS (pre-order, sorted
/// siblings) walk order. Uses component-by-component comparison: at the
/// first differing level, the smaller component wins. A shorter path
/// (directory/ancestor) comes before a longer path that extends it.
pub(crate) fn dfs_after(a: &str, b: &str) -> bool {
    let a_parts: Vec<&str> = a.split('/').filter(|s| !s.is_empty()).collect();
    let b_parts: Vec<&str> = b.split('/').filter(|s| !s.is_empty()).collect();

    for (ap, bp) in a_parts.iter().zip(b_parts.iter()) {
        if ap != bp {
            return ap < bp; // a's component is smaller → a comes first → b is after a
        }
    }
    // All common prefixes match: the shorter path (ancestor) comes first in DFS
    a_parts.len() < b_parts.len()
}

/// Read a full snapshot from a file path (or "-" for stdin) into a path → record index.
pub fn read_snapshot(path: &str) -> Result<Vec<FileRecord>> {
    let mut reader = BufReader::new(open_snapshot_reader(path)?);
    check_snapshot_version(&mut reader)?;
    read_all_records(&mut reader)
}

fn open_snapshot_reader(path: &str) -> Result<Box<dyn Read>> {
    if path == "-" {
        Ok(Box::new(std::io::stdin()))
    } else {
        Ok(Box::new(std::fs::File::open(path)?))
    }
}

fn check_snapshot_version(reader: &mut BufReader<Box<dyn Read>>) -> Result<()> {
    let mut version_buf = [0u8; 2];
    reader
        .read_exact(&mut version_buf)
        .context("read version")?;
    let version = u16::from_le_bytes(version_buf);
    if version > VERSION {
        bail!("unsupported snapshot version {}", version);
    }
    Ok(())
}

// Read records until trailer or EOF, validating DFS order as we go.
fn read_all_records(reader: &mut BufReader<Box<dyn Read>>) -> Result<Vec<FileRecord>> {
    let mut records: Vec<FileRecord> = Vec::new();
    let mut count = 0u64;
    let mut trailer_count: Option<u64> = None;
    while let Some(item) = read_record(reader)? {
        match item {
            ReadItem::Record(record) => {
                count += 1;
                let record: FileRecord = *record;
                check_dfs_order(&records, &record)?;
                records.push(record);
            }
            ReadItem::Trailer(file_count) => {
                trailer_count = Some(file_count);
                break;
            }
        }
    }
    check_trailer_count(count, trailer_count);
    Ok(records)
}

fn check_dfs_order(records: &[FileRecord], record: &FileRecord) -> Result<()> {
    if let Some(prev) = records.last() {
        if !dfs_after(&prev.path, &record.path) {
            bail!(
                "snapshot records are not in DFS order at \"{}\" (previous: \"{}\")",
                record.path,
                prev.path
            );
        }
    }
    Ok(())
}

fn check_trailer_count(count: u64, trailer_count: Option<u64>) {
    if let Some(expected) = trailer_count {
        if count != expected {
            eprintln!(
                "warning: snapshot has {} records but trailer says {} (duplicates or corruption?)",
                count, expected
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::FileRecord;
    use std::io::Cursor;

    fn roundtrip(records: Vec<FileRecord>) -> Vec<FileRecord> {
        let mut buf = Vec::new();
        write_header(&mut buf).unwrap();
        for r in &records {
            write_record(r, &mut buf).unwrap();
        }
        write_trailer(records.len() as u64, &mut buf).unwrap();

        // Read back
        let mut cursor = Cursor::new(buf);
        let mut version_buf = [0u8; 2];
        cursor.read_exact(&mut version_buf).unwrap();
        assert_eq!(u16::from_le_bytes(version_buf), VERSION);

        let mut out = Vec::new();
        while let Some(ReadItem::Record(record)) = read_record(&mut cursor).unwrap() {
            let record: FileRecord = *record;
            out.push(record);
        }
        out
    }

    #[test]
    fn roundtrip_regular_file() {
        let record = FileRecord {
            path: "/etc/hostname".into(),
            file_type: FileType::Regular,
            mode: 0o644,
            user: "root".into(),
            group: "root".into(),
            hardlinks: 1,
            size: Some(1234),
            mtime: Some(1700000000),
            checksum: Some(vec![0xde, 0xad, 0xbe, 0xef]),
            ..Default::default()
        };
        let out = roundtrip(vec![record.clone()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], record);
    }

    #[test]
    fn roundtrip_symlink() {
        let record = FileRecord {
            path: "/usr/bin/sh".into(),
            file_type: FileType::Symlink,
            symlink_target: Some("/bin/bash".into()),
            ..Default::default()
        };
        let out = roundtrip(vec![record.clone()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].file_type, FileType::Symlink);
        assert_eq!(out[0].symlink_target, Some("/bin/bash".into()));
    }

    #[test]
    fn roundtrip_device() {
        let record = FileRecord {
            path: "/dev/null".into(),
            file_type: FileType::CharDev,
            dev_major: Some(1),
            dev_minor: Some(3),
            ..Default::default()
        };
        let out = roundtrip(vec![record.clone()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].file_type, FileType::CharDev);
        assert_eq!(out[0].dev_major, Some(1));
        assert_eq!(out[0].dev_minor, Some(3));
    }

    #[test]
    fn roundtrip_with_xattrs() {
        let record = FileRecord {
            path: "/etc/special".into(),
            xattrs: vec![
                ("security.selinux".into(), b"unconfined_u".to_vec()),
                ("user.comment".into(), b"hello".to_vec()),
            ],
            ..Default::default()
        };
        let out = roundtrip(vec![record.clone()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].xattrs.len(), 2);
        assert_eq!(out[0].xattrs[0].0, "security.selinux");
        assert_eq!(out[0].xattrs[1].0, "user.comment");
    }

    #[test]
    fn roundtrip_with_file_attrs() {
        let record = FileRecord {
            path: "/etc/selinux/config".into(),
            file_attrs: "i".into(),
            ..Default::default()
        };
        let out = roundtrip(vec![record.clone()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].file_attrs, "i");
    }

    #[test]
    fn roundtrip_checksum_skipped() {
        let record = FileRecord {
            path: "/big".into(),
            checksum_skipped: true,
            size: Some(999999),
            ..Default::default()
        };
        let out = roundtrip(vec![record.clone()]);
        assert_eq!(out.len(), 1);
        assert!(out[0].checksum_skipped);
        assert!(out[0].checksum.is_none());
    }

    #[test]
    fn roundtrip_multiple_records() {
        let records = vec![
            FileRecord {
                path: "/a".into(),
                ..Default::default()
            },
            FileRecord {
                path: "/b".into(),
                mode: 0o755,
                ..Default::default()
            },
            FileRecord {
                path: "/c".into(),
                file_type: FileType::Directory,
                ..Default::default()
            },
        ];
        let out = roundtrip(records);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].path, "/a");
        assert_eq!(out[1].mode, 0o755);
        assert_eq!(out[2].file_type, FileType::Directory);
    }

    #[test]
    fn roundtrip_empty_snapshot() {
        let out = roundtrip(vec![]);
        assert!(out.is_empty());
    }

    #[test]
    fn roundtrip_directory_type() {
        let record = FileRecord {
            path: "/etc".into(),
            file_type: FileType::Directory,
            mode: 0o755,
            ..Default::default()
        };
        let out = roundtrip(vec![record.clone()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].file_type, FileType::Directory);
    }

    #[test]
    fn read_snapshot_rejects_out_of_dfs_order() {
        // Write records in invalid DFS order: child before parent
        // Valid DFS: /etc, /etc/journal, /etc/journal.conf
        // Invalid:   /etc/journal.conf, /etc/journal  (file before its sibling dir)
        let mut buf = Vec::new();
        write_header(&mut buf).unwrap();
        write_record(
            &FileRecord {
                path: "/etc/journal.conf".into(),
                ..Default::default()
            },
            &mut buf,
        )
        .unwrap();
        write_record(
            &FileRecord {
                path: "/etc/journal".into(),
                file_type: FileType::Directory,
                ..Default::default()
            },
            &mut buf,
        )
        .unwrap();
        write_trailer(2, &mut buf).unwrap();

        // Write to a temp file and try to read
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.fs");
        std::fs::write(&path, &buf).unwrap();

        let result = read_snapshot(path.to_str().unwrap());
        assert!(result.is_err());
        let err = format!("{}", result.err().unwrap());
        assert!(
            err.contains("not in DFS order"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn read_snapshot_rejects_duplicate_paths() {
        // Same path twice is not strictly increasing DFS order
        let mut buf = Vec::new();
        write_header(&mut buf).unwrap();
        write_record(
            &FileRecord {
                path: "/a".into(),
                ..Default::default()
            },
            &mut buf,
        )
        .unwrap();
        write_record(
            &FileRecord {
                path: "/a".into(),
                ..Default::default()
            },
            &mut buf,
        )
        .unwrap();
        write_trailer(2, &mut buf).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dup.fs");
        std::fs::write(&path, &buf).unwrap();

        let result = read_snapshot(path.to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn dfs_after_basic() {
        // Ancestor comes before descendant
        assert!(dfs_after("/etc", "/etc/journal"));
        assert!(dfs_after("/etc/journal", "/etc/journal/profile"));
        // Sibling: smaller component first
        assert!(dfs_after("/etc/a", "/etc/b"));
        // Dir subtree before sibling file (DFS: dir contents, then file)
        assert!(dfs_after("/etc/journal", "/etc/journal.conf"));
        assert!(dfs_after("/etc/journal/profile", "/etc/journal.conf"));
        // Reverse: not after
        assert!(!dfs_after("/etc/journal.conf", "/etc/journal"));
        assert!(!dfs_after("/etc/journal", "/etc"));
        // Equal: not after (strict)
        assert!(!dfs_after("/a", "/a"));
    }
}
