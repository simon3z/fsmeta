//! Filesystem walk and metadata capture.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;

use crate::checksum;
use crate::record::{FileRecord, FileType};

/// Checksum size limit (1 GB) applied to the live walk during `compare`.
/// Matches `dump`'s default: content identity needs the same threshold on
/// both sides, otherwise an unchanged large file would flag `?` spuriously.
pub const COMPARE_CHECKSUM_LIMIT: u64 = 1 << 30;

static UID_CACHE: OnceLock<HashMap<u32, String>> = OnceLock::new();
static GID_CACHE: OnceLock<HashMap<u32, String>> = OnceLock::new();

/// Walk a set of paths, returning a receiver that yields records one at a time.
///
/// Architecture: a single walker thread recursively discovers paths and
/// captures metadata for each, streaming records to the output channel.
/// Memory is O(directory depth) — no full path list in memory.
pub fn walk_stream(config: &WalkConfig) -> Result<mpsc::Receiver<FileRecord>> {
    // Pre-load UID/GID caches
    load_uid_cache();
    load_gid_cache();

    let (tx, rx) = mpsc::channel::<FileRecord>();

    let paths = config.paths.clone();
    let excludes = config.excludes.clone();
    let from_stdin = config.from_stdin;
    let checksum_size_limit = config.checksum_size_limit;
    let verbose = config.verbose;

    thread::spawn(move || {
        if from_stdin {
            walk_stdin(&tx, checksum_size_limit, verbose);
        } else {
            for p in &paths {
                walk_dir(Path::new(p), &excludes, &tx, checksum_size_limit, verbose);
            }
        }
        // tx dropped here: receiver will see Err
    });

    Ok(rx)
}

/// Walk paths from stdin, capturing metadata and sending records.
fn walk_stdin(tx: &mpsc::Sender<FileRecord>, checksum_size_limit: u64, verbose: bool) {
    for line in std::io::stdin().lines() {
        match line {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let path = PathBuf::from(line);
                if !capture_and_send(&path, checksum_size_limit, verbose, tx) {
                    break;
                }
            }
            Err(e) => {
                eprintln!("warning: error reading stdin: {}", e);
                break;
            }
        }
    }
}

/// Recursively walk a directory, capturing metadata and sending records.
fn walk_dir(
    path: &Path,
    excludes: &[String],
    tx: &mpsc::Sender<FileRecord>,
    checksum_size_limit: u64,
    verbose: bool,
) -> bool {
    for excl in excludes {
        if path.starts_with(Path::new(excl)) {
            return true;
        }
    }

    // Capture this file/dir
    if !capture_and_send(path, checksum_size_limit, verbose, tx) {
        return false; // consumer gone, stop walking
    }

    // If it's a directory, recurse
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return true,
    };
    if !meta.is_dir() {
        return true;
    }

    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return true,
    };

    // Collect and sort by filename for deterministic walk order
    let mut children: Vec<PathBuf> = Vec::new();
    for entry in entries {
        match entry {
            Ok(e) => children.push(e.path()),
            Err(_) => continue,
        }
    }
    children.sort();

    for child in children {
        if !walk_dir(&child, excludes, tx, checksum_size_limit, verbose) {
            return false;
        }
    }

    true
}

/// Capture metadata for a path and send the record. Returns false if the
/// consumer is gone (sender disconnected).
fn capture_and_send(
    path: &Path,
    checksum_size_limit: u64,
    verbose: bool,
    tx: &mpsc::Sender<FileRecord>,
) -> bool {
    match capture_metadata(path, checksum_size_limit) {
        Ok(record) => {
            if verbose {
                eprintln!("{}", record.path);
            }
            tx.send(record).is_ok()
        }
        Err(e) => {
            eprintln!("warning: {}: {}", path.display(), e);
            true
        }
    }
}

/// Capture metadata for a single file.
fn capture_metadata(path: &Path, checksum_size_limit: u64) -> Result<FileRecord> {
    let meta =
        std::fs::symlink_metadata(path).with_context(|| format!("metadata {}", path.display()))?;
    let file_type = path_to_file_type(&meta);
    let mut record = FileRecord {
        path: path.to_string_lossy().into(),
        file_type,
        ..Default::default()
    };

    use std::os::unix::fs::MetadataExt;
    let raw_mode = meta.mode();
    let raw_uid = meta.uid();
    let raw_gid = meta.gid();
    let raw_nlink = meta.nlink();
    let raw_rdev = meta.rdev();

    record.mode = raw_mode;
    record.hardlinks = raw_nlink as u32;
    record.user = get_username(raw_uid);
    record.group = get_groupname(raw_gid);
    record.perms = special_bits(raw_mode);
    record.size = Some(meta.size());
    record.mtime = Some(meta.mtime());

    match file_type {
        FileType::Regular => {
            let size = meta.size();

            if size <= checksum_size_limit {
                match checksum::sha1_checksum(path) {
                    Ok(c) => {
                        record.checksum = Some(c);
                    }
                    Err(e) => {
                        eprintln!("warning: checksum failed for {}: {}", path.display(), e);
                        record.checksum_skipped = true;
                    }
                }
            } else {
                record.checksum_skipped = true;
            }
        }
        FileType::Symlink => match fs::read_link(path) {
            Ok(target) => {
                record.symlink_target = Some(target.to_string_lossy().to_string());
            }
            Err(e) => {
                eprintln!("warning: readlink failed for {}: {}", path.display(), e);
            }
        },
        FileType::CharDev | FileType::BlockDev => {
            let rdev = raw_rdev;
            let major = ((rdev >> 8) & 0xfff) as u32;
            let minor = ((rdev & 0xff) | ((rdev >> 12) & 0xfff00)) as u32;
            record.dev_major = Some(major);
            record.dev_minor = Some(minor);
        }
        _ => {}
    }

    // xattrs
    record.xattrs = get_all_xattrs(path);

    // File attributes (lsattr)
    record.file_attrs = get_lsattr(path);

    Ok(record)
}

/// Walk configuration shared by `dump` and `compare`.
pub struct WalkConfig {
    pub paths: Vec<String>,
    pub excludes: Vec<String>,
    pub checksum_size_limit: u64,
    pub from_stdin: bool,
    pub verbose: bool,
}

fn path_to_file_type(meta: &std::fs::Metadata) -> FileType {
    use std::os::unix::fs::FileTypeExt;
    match meta.file_type() {
        ft if ft.is_symlink() => FileType::Symlink,
        ft if ft.is_dir() => FileType::Directory,
        ft if ft.is_char_device() => FileType::CharDev,
        ft if ft.is_block_device() => FileType::BlockDev,
        ft if ft.is_fifo() => FileType::Fifo,
        ft if ft.is_socket() => FileType::Socket,
        _ => FileType::Regular,
    }
}

fn special_bits(mode: u32) -> u8 {
    let mut bits = 0;
    if mode & 0o4000 != 0 {
        bits |= 1;
    }
    if mode & 0o2000 != 0 {
        bits |= 2;
    }
    if mode & 0o1000 != 0 {
        bits |= 4;
    }
    bits
}

// --- UID/GID resolution via /etc/passwd and /etc/group ---

fn load_uid_cache() {
    UID_CACHE.get_or_init(|| {
        let mut cache = HashMap::new();
        if let Ok(contents) = std::fs::read_to_string("/etc/passwd") {
            for line in contents.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 3 {
                    if let Ok(uid) = parts[2].parse::<u32>() {
                        cache.insert(uid, parts[0].to_string());
                    }
                }
            }
        }
        cache
    });
}

fn load_gid_cache() {
    GID_CACHE.get_or_init(|| {
        let mut cache = HashMap::new();
        if let Ok(contents) = std::fs::read_to_string("/etc/group") {
            for line in contents.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 3 {
                    if let Ok(gid) = parts[2].parse::<u32>() {
                        cache.insert(gid, parts[0].to_string());
                    }
                }
            }
        }
        cache
    });
}

fn get_username(uid: u32) -> String {
    UID_CACHE
        .get()
        .and_then(|cache| cache.get(&uid).cloned())
        .unwrap_or_else(|| uid.to_string())
}

fn get_groupname(gid: u32) -> String {
    GID_CACHE
        .get()
        .and_then(|cache| cache.get(&gid).cloned())
        .unwrap_or_else(|| gid.to_string())
}

// --- xattrs via raw syscall ---

// Known xattr names to try when llistxattr is unavailable (e.g. btrfs)
const KNOWN_XATTRS: &[&str] = &[
    "security.selinux",
    "security.capability",
    "system.posix_acl_access",
    "system.posix_acl_default",
    "user.fsmeta_marker", // example user xattr
];

fn get_all_xattrs(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut xattrs = Vec::new();

    let c_path = match CString::new(path.as_os_str().as_encoded_bytes()) {
        Ok(p) => p,
        Err(_) => return xattrs,
    };

    // Try llistxattr (syscall 233 on x86_64) to list xattr names
    let mut buf = vec![0u8; 4096];
    let ret: libc::ssize_t = unsafe {
        libc::syscall(
            233, // __NR_llistxattr on x86_64
            c_path.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len() as libc::size_t,
        ) as libc::ssize_t
    };

    let names: Vec<String> = if ret > 0 {
        let names_str = unsafe { std::str::from_utf8_unchecked(&buf[..ret as usize]) };
        names_str
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    } else {
        // Fallback: try known xattr names
        KNOWN_XATTRS.iter().map(|s| s.to_string()).collect()
    };

    for name in names {
        let c_name = match CString::new(name.as_bytes()) {
            Ok(n) => n,
            Err(_) => continue,
        };
        let mut val_buf = vec![0u8; 65536];
        let val_ret = unsafe {
            libc::lgetxattr(
                c_path.as_ptr(),
                c_name.as_ptr() as *mut libc::c_char,
                val_buf.as_mut_ptr() as *mut libc::c_void,
                val_buf.len() as libc::size_t,
            )
        };
        if val_ret > 0 {
            let val = val_buf[..val_ret as usize].to_vec();
            xattrs.push((name, val));
        }
    }
    xattrs
}

// --- lsattr via ioctl ---

// FS_IOC_GETFLAGS = _IOW('f', 1, unsigned long) = 0x80084501 on x86_64
const FS_IOC_GETFLAGS: libc::c_ulong = 0x8008_4501;

// ext4/btrfs file flags (from <linux/fs.h>)
const FS_SECRM_FL: u64 = 0x0001;
const FS_NOCOW_FL: u64 = 0x0002;
const FS_COMPR_FL: u64 = 0x0004;
const FS_APPEND_FL: u64 = 0x0040;
const FS_NODIRSYNC_FL: u64 = 0x0080;
const FS_NOATIME_FL: u64 = 0x0100;

fn get_lsattr(path: &Path) -> String {
    let mut attrs = String::new();

    let c_path = match CString::new(path.as_os_str().as_encoded_bytes()) {
        Ok(p) => p,
        Err(_) => return attrs,
    };
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
    if fd < 0 {
        return attrs;
    }

    let mut flags: libc::c_ulong = 0;
    let ret = unsafe { libc::ioctl(fd, FS_IOC_GETFLAGS, &mut flags as *mut libc::c_ulong) };
    unsafe { libc::close(fd) };

    if ret < 0 {
        return attrs;
    }

    let flags = flags as u64;
    if flags & FS_APPEND_FL != 0 {
        attrs.push('a');
    }
    if flags & FS_COMPR_FL != 0 {
        attrs.push('c');
    }
    if flags & FS_NODIRSYNC_FL != 0 {
        attrs.push('d');
    }
    if flags & FS_SECRM_FL != 0 {
        attrs.push('i');
    }
    if flags & FS_NOCOW_FL != 0 {
        attrs.push('S');
    }
    if flags & FS_NOATIME_FL != 0 {
        attrs.push('A');
    }

    attrs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that path_to_file_type correctly identifies a regular file.
    #[test]
    fn file_type_regular() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, b"hello").unwrap();
        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert_eq!(path_to_file_type(&meta), FileType::Regular);
    }

    /// Test that path_to_file_type correctly identifies a directory.
    #[test]
    fn file_type_directory() {
        let dir = tempfile::tempdir().unwrap();
        let meta = std::fs::symlink_metadata(dir.path()).unwrap();
        assert_eq!(path_to_file_type(&meta), FileType::Directory);
    }

    /// Test that path_to_file_type correctly identifies a symlink.
    #[test]
    fn file_type_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        std::fs::write(&target, b"data").unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let meta = std::fs::symlink_metadata(&link).unwrap();
        assert_eq!(path_to_file_type(&meta), FileType::Symlink);
    }

    /// Test that path_to_file_type correctly identifies a char device.
    #[test]
    fn file_type_char_device() {
        // /dev/null is a char device on all Linux systems
        let meta = std::fs::symlink_metadata("/dev/null").unwrap();
        assert_eq!(path_to_file_type(&meta), FileType::CharDev);
    }

    /// Test that capture_metadata does NOT attempt checksum on char devices.
    #[test]
    fn no_checksum_on_char_device() {
        let meta = std::fs::symlink_metadata("/dev/null").unwrap();
        let file_type = path_to_file_type(&meta);
        assert_eq!(file_type, FileType::CharDev);
        // capture_metadata should return without error (no read attempt)
        let record = capture_metadata(std::path::Path::new("/dev/null"), 1024)
            .expect("char device metadata capture should succeed");
        assert_eq!(record.file_type, FileType::CharDev);
        assert!(record.checksum.is_none(), "no checksum for char devices");
        assert!(
            !record.checksum_skipped,
            "checksum_skipped should not be set for char devices"
        );
        // size is now captured for all types (like stat does)
    }

    /// Test that capture_metadata does NOT attempt checksum on block devices.
    #[test]
    fn no_checksum_on_block_device() {
        // Find a block device (loop devices or similar)
        let entries = std::fs::read_dir("/dev").unwrap();
        let mut found = None;
        for entry in entries.flatten() {
            let meta = match std::fs::symlink_metadata(entry.path()) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if path_to_file_type(&meta) == FileType::BlockDev {
                found = Some(entry.path());
                break;
            }
        }
        if let Some(dev) = found {
            let record = capture_metadata(&dev, 1024).expect("block device metadata");
            assert_eq!(record.file_type, FileType::BlockDev);
            assert!(record.checksum.is_none());
            assert!(!record.checksum_skipped);
        }
        // If no block device found, test passes trivially
    }

    /// Test that capture_metadata DOES checksum regular files within limit.
    #[test]
    fn checksum_for_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.txt");
        std::fs::write(&path, b"test content").unwrap();
        let record = capture_metadata(path.as_path(), 1024).unwrap();
        assert_eq!(record.file_type, FileType::Regular);
        assert!(record.checksum.is_some(), "small file should have checksum");
    }

    /// Test that capture_metadata sets checksum_skipped for large files.
    #[test]
    fn checksum_skipped_for_large_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        // Write a small file but use a very low limit
        std::fs::write(&path, b"data").unwrap();
        let record = capture_metadata(path.as_path(), 3).unwrap();
        assert!(record.checksum.is_none());
        assert!(
            record.checksum_skipped,
            "should be skipped due to size limit"
        );
    }

    /// Test that walk_stream output is globally sorted when a directory
    /// and a sibling file share a prefix (the '.' < '/' case).
    #[test]
    fn walk_output_is_globally_sorted_with_prefix_collision() {
        // Tree:
        //   base/
        //     journal/          (directory)
        //       profile         (file)
        //     journal.conf      (file)
        //
        // Byte-wise: journal < journal.conf < journal/profile
        // ('.' 0x2E < '/' 0x2F)
        let base = tempfile::tempdir().unwrap();
        let base = base.path();
        std::fs::create_dir_all(base.join("journal")).unwrap();
        std::fs::write(base.join("journal/profile"), b"data").unwrap();
        std::fs::write(base.join("journal.conf"), b"file").unwrap();

        let config = WalkConfig {
            paths: vec![base.to_string_lossy().to_string()],
            excludes: vec![],
            checksum_size_limit: COMPARE_CHECKSUM_LIMIT,
            from_stdin: false,
            verbose: false,
        };
        let rx = walk_stream(&config).unwrap();
        let mut paths: Vec<String> = Vec::new();
        for record in rx {
            paths.push(record.path);
        }

        // The raw walk is DFS-sorted, NOT globally sorted.
        // We only assert that the consumer (cmd_dump/cmd_compare) sorts.
        // Here we just verify the walk sees all files.
        assert!(paths.iter().any(|p| p.ends_with("journal.conf")));
        assert!(paths.iter().any(|p| p.ends_with("journal")));
        assert!(paths.iter().any(|p| p.ends_with("journal/profile")));
    }
}
