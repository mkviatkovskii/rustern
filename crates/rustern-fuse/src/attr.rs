//! File attributes and their kernel TTLs from stat responses (data-model.md "Attributes").

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{FileAttr, FileType, INodeNo};
use rustern_core::inode::{InodeKind, TernInode};
use rustern_msgs::{StatDirectoryResp, StatFileResp, TernTime};

/// Mode bits of every inode (as ternfuse): read and execute for everyone.
pub const PERM: u16 = 0o555;

/// Block size reported to `stat`.
const BLKSIZE: u32 = 4096;

/// Settings that shape attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttrConfig {
    /// `-attr-cache-time`: TTL of file attributes.
    pub attr_ttl: Duration,
    /// `-dir-entry-cache-time`: TTL of directory entries; caps directory attributes
    /// (DEV-003).
    pub entry_ttl: Duration,
    /// Owner of every inode.
    pub uid: u32,
    /// Group of every inode.
    pub gid: u32,
}

impl AttrConfig {
    /// Owner 0:0, or the daemon's effective ids with `-set-uid`.
    pub fn new(attr_ttl: Duration, entry_ttl: Duration, set_uid: bool) -> Self {
        let (uid, gid) = if set_uid {
            (
                rustix::process::geteuid().as_raw(),
                rustix::process::getegid().as_raw(),
            )
        } else {
            (0, 0)
        };
        Self {
            attr_ttl,
            entry_ttl,
            uid,
            gid,
        }
    }

    /// TTL of a directory's attributes: `min(attr TTL, entry TTL)` (DEV-003).
    pub fn dir_ttl(&self) -> Duration {
        self.attr_ttl.min(self.entry_ttl)
    }

    /// Attributes of a file or symlink, and their TTL.
    pub fn file(&self, id: TernInode, st: &StatFileResp) -> (FileAttr, Duration) {
        let mut a = self.base(id, st.mtime);
        a.size = st.size;
        a.blocks = st.size.div_ceil(512);
        a.atime = time(st.atime);
        (a, self.attr_ttl)
    }

    /// Attributes of a directory, and their TTL.
    pub fn dir(&self, id: TernInode, st: &StatDirectoryResp) -> (FileAttr, Duration) {
        (self.base(id, st.mtime), self.dir_ttl())
    }

    /// Attributes of a negative entry (ino 0): the kernel caches "no such name".
    pub fn negative(&self) -> FileAttr {
        let mut a = self.base(TernInode::ROOT, TernTime(0));
        a.ino = INodeNo(0);
        a
    }

    fn base(&self, id: TernInode, mtime: TernTime) -> FileAttr {
        let mtime = time(mtime);
        FileAttr {
            ino: INodeNo(id.to_fuse()),
            size: 0,
            blocks: 0,
            atime: mtime,
            mtime,
            ctime: mtime,
            crtime: mtime,
            kind: kind(id.kind()),
            perm: PERM,
            nlink: 1,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: BLKSIZE,
            flags: 0,
        }
    }
}

/// FUSE file type of an inode kind.
pub fn kind(k: InodeKind) -> FileType {
    match k {
        InodeKind::Directory => FileType::Directory,
        InodeKind::File => FileType::RegularFile,
        InodeKind::Symlink => FileType::Symlink,
    }
}

/// A TernTime (ns since the epoch) as a `SystemTime`.
pub fn time(t: TernTime) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_nanos(t.0))
        .unwrap_or(UNIX_EPOCH)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects)] // test code
mod tests {
    use rustern_msgs::InodeId;

    use super::*;

    const FILE: u64 = (2 << 61) | (9 << 8) | 4;
    const LINK: u64 = (3 << 61) | (9 << 8) | 4;
    const DIR: u64 = (1 << 61) | (9 << 8) | 4;

    fn cfg() -> AttrConfig {
        AttrConfig::new(Duration::from_secs(3600), Duration::from_millis(250), false)
    }

    fn ino(id: u64) -> TernInode {
        TernInode::new(id).unwrap()
    }

    #[test]
    fn file_attributes() {
        let st = StatFileResp {
            mtime: TernTime(1_700_000_000_123_456_789),
            atime: TernTime(1_700_000_100_000_000_001),
            size: 5_120_001,
        };
        let (a, ttl) = cfg().file(ino(FILE), &st);
        assert_eq!(a.ino, INodeNo(FILE));
        assert_eq!(a.kind, FileType::RegularFile);
        assert_eq!(a.size, 5_120_001);
        assert_eq!(a.blocks, 5_120_001_u64.div_ceil(512));
        assert_eq!(
            a.mtime,
            UNIX_EPOCH + Duration::from_nanos(1_700_000_000_123_456_789)
        );
        assert_eq!(
            a.atime,
            UNIX_EPOCH + Duration::from_nanos(1_700_000_100_000_000_001)
        );
        assert_eq!(a.ctime, a.mtime);
        assert_eq!((a.perm, a.nlink, a.uid, a.gid), (0o555, 1, 0, 0));
        assert_eq!(ttl, Duration::from_secs(3600));

        let (l, _) = cfg().file(ino(LINK), &st);
        assert_eq!(l.kind, FileType::Symlink);
    }

    #[test]
    fn directory_attributes() {
        let st = StatDirectoryResp {
            mtime: TernTime(42_000_000_007),
            owner: InodeId(1 << 61),
            ..StatDirectoryResp::default()
        };
        let (a, ttl) = cfg().dir(ino(DIR), &st);
        assert_eq!((a.kind, a.size, a.blocks), (FileType::Directory, 0, 0));
        assert_eq!(a.mtime, UNIX_EPOCH + Duration::from_nanos(42_000_000_007));
        assert_eq!((a.atime, a.ctime), (a.mtime, a.mtime));
        assert_eq!((a.perm, a.nlink), (0o555, 1));
        // DEV-003: min(attr TTL, entry TTL).
        assert_eq!(ttl, Duration::from_millis(250));
        let short = AttrConfig::new(Duration::from_millis(100), Duration::from_secs(1), false);
        assert_eq!(short.dir(ino(DIR), &st).1, Duration::from_millis(100));

        let (root, _) = cfg().dir(TernInode::ROOT, &st);
        assert_eq!(root.ino, INodeNo(1));
    }

    #[test]
    fn set_uid_reports_effective_ids() {
        let c = AttrConfig::new(Duration::ZERO, Duration::ZERO, true);
        assert_eq!(c.uid, rustix::process::geteuid().as_raw());
        assert_eq!(c.gid, rustix::process::getegid().as_raw());
        let st = StatFileResp::default();
        let (a, _) = c.file(ino(FILE), &st);
        assert_eq!((a.uid, a.gid), (c.uid, c.gid));
    }
}
