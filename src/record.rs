//! File metadata record.

/// File type (matches u8 encoding in format).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileType {
    #[default]
    Regular = 0,
    Symlink = 1,
    Directory = 2,
    CharDev = 3,
    BlockDev = 4,
    Fifo = 5,
    Socket = 6,
}

impl From<u8> for FileType {
    fn from(v: u8) -> Self {
        match v {
            0 => FileType::Regular,
            1 => FileType::Symlink,
            2 => FileType::Directory,
            3 => FileType::CharDev,
            4 => FileType::BlockDev,
            5 => FileType::Fifo,
            6 => FileType::Socket,
            _ => FileType::Regular,
        }
    }
}

impl FileType {
    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

/// A single file's metadata record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileRecord {
    pub path: String,
    pub file_type: FileType,
    pub mode: u32,
    pub user: String,
    pub group: String,
    pub perms: u8,
    pub hardlinks: u32,
    // Regular files:
    pub size: Option<u64>,
    pub mtime: Option<i64>,
    pub checksum: Option<Vec<u8>>,
    pub checksum_skipped: bool,
    // Symlinks:
    pub symlink_target: Option<String>,
    // Char/block devices:
    pub dev_major: Option<u32>,
    pub dev_minor: Option<u32>,
    // All files:
    pub xattrs: Vec<(String, Vec<u8>)>,
    pub file_attrs: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetype_u8_roundtrip() {
        for t in [
            FileType::Regular,
            FileType::Symlink,
            FileType::Directory,
            FileType::CharDev,
            FileType::BlockDev,
            FileType::Fifo,
            FileType::Socket,
        ] {
            assert_eq!(FileType::from(t.to_u8()), t);
        }
    }

    #[test]
    fn filetype_unknown_defaults_to_regular() {
        assert_eq!(FileType::from(255), FileType::Regular);
    }

    #[test]
    fn filetype_default_is_regular() {
        assert_eq!(FileType::default(), FileType::Regular);
    }

    #[test]
    fn record_default_is_regular_empty() {
        let r = FileRecord::default();
        assert_eq!(r.file_type, FileType::Regular);
        assert!(r.path.is_empty());
        assert!(r.size.is_none());
        assert!(r.checksum.is_none());
        assert!(!r.checksum_skipped);
    }
}
