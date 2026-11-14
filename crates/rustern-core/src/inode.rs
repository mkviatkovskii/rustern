//! TernFS inode ids (data-model.md "InodeId"): top 3 bits are the type
//! (`DIRECTORY` = 1, `FILE` = 2, `SYMLINK` = 3), the low byte is the shard. FUSE reserves
//! ino 1 for the root, so the root id (`1 << 61`) maps to 1 and every other id to itself.

use rustern_msgs::{
    INODE_TYPE_DIRECTORY, INODE_TYPE_FILE, INODE_TYPE_SYMLINK, InodeId, ROOT_DIR_INODE_ID,
};

/// FUSE inode number of the root directory.
pub const FUSE_ROOT_ID: u64 = 1;

/// Kind of a TernFS inode, from its type bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InodeKind {
    /// Type bits 1.
    Directory,
    /// Type bits 2.
    File,
    /// Type bits 3.
    Symlink,
}

/// An inode id whose type bits are invalid, or FUSE ino 0 (mapped to EIO).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("malformed inode id {0:#x}")]
pub struct InvalidInode(pub u64);

/// A validated TernFS inode id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TernInode(InodeId);

impl TernInode {
    /// The root directory.
    pub const ROOT: Self = Self(ROOT_DIR_INODE_ID);

    /// Validates the type bits of `id`.
    pub fn new(id: u64) -> Result<Self, InvalidInode> {
        match type_bits(id) {
            INODE_TYPE_DIRECTORY | INODE_TYPE_FILE | INODE_TYPE_SYMLINK => Ok(Self(InodeId(id))),
            _ => Err(InvalidInode(id)),
        }
    }

    /// The wire id.
    pub const fn id(self) -> InodeId {
        self.0
    }

    /// Shard that owns the inode.
    pub fn shard(self) -> u8 {
        self.0.0.to_le_bytes()[0]
    }

    /// Kind from the type bits.
    pub fn kind(self) -> InodeKind {
        match type_bits(self.0.0) {
            INODE_TYPE_DIRECTORY => InodeKind::Directory,
            INODE_TYPE_FILE => InodeKind::File,
            _ => InodeKind::Symlink, // validated by `new`
        }
    }

    /// FUSE inode number.
    pub fn to_fuse(self) -> u64 {
        if self == Self::ROOT {
            FUSE_ROOT_ID
        } else {
            self.0.0
        }
    }

    /// Inode for a FUSE inode number.
    pub fn from_fuse(ino: u64) -> Result<Self, InvalidInode> {
        if ino == FUSE_ROOT_ID {
            Ok(Self::ROOT)
        } else {
            Self::new(ino)
        }
    }
}

const fn type_bits(id: u64) -> u8 {
    id.to_be_bytes()[0] >> 5
}

impl TryFrom<InodeId> for TernInode {
    type Error = InvalidInode;
    fn try_from(id: InodeId) -> Result<Self, InvalidInode> {
        Self::new(id.0)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const fn raw(kind: u8, n: u64, shard: u8) -> u64 {
        ((kind as u64) << 61) | (n << 8) | shard as u64
    }

    #[test]
    fn type_bits_and_shard() {
        let d = TernInode::new(raw(INODE_TYPE_DIRECTORY, 5, 7)).unwrap();
        assert_eq!((d.kind(), d.shard()), (InodeKind::Directory, 7));
        let f = TernInode::new(raw(INODE_TYPE_FILE, 5, 255)).unwrap();
        assert_eq!((f.kind(), f.shard()), (InodeKind::File, 255));
        let s = TernInode::new(raw(INODE_TYPE_SYMLINK, 0, 0)).unwrap();
        assert_eq!((s.kind(), s.shard()), (InodeKind::Symlink, 0));
        assert_eq!(TernInode::ROOT.kind(), InodeKind::Directory);
        assert_eq!(TernInode::ROOT.shard(), 0);
    }

    #[test]
    fn invalid_type_bits_rejected() {
        for t in [0u8, 4, 5, 6, 7] {
            let id = raw(t, 1, 1);
            assert_eq!(TernInode::new(id), Err(InvalidInode(id)), "type {t}");
        }
    }

    #[test]
    fn fuse_mapping() {
        assert_eq!(TernInode::ROOT.to_fuse(), FUSE_ROOT_ID);
        assert_eq!(TernInode::from_fuse(FUSE_ROOT_ID), Ok(TernInode::ROOT));
        assert_eq!(ROOT_DIR_INODE_ID.0, 1 << 61);
        let f = TernInode::new(raw(INODE_TYPE_FILE, 9, 3)).unwrap();
        assert_eq!(f.to_fuse(), f.id().0);
        assert_eq!(TernInode::from_fuse(0), Err(InvalidInode(0)));
        assert!(TernInode::from_fuse(2).is_err(), "type bits 0");
    }

    proptest! {
        #[test]
        fn fuse_round_trip(kind in 1u8..=3, n in 0u64..(1 << 53), shard: u8) {
            let id = TernInode::new(raw(kind, n, shard)).unwrap();
            prop_assert_eq!(TernInode::from_fuse(id.to_fuse()), Ok(id));
        }

        #[test]
        fn new_never_panics(id: u64) {
            let _ = TernInode::new(id);
            let _ = TernInode::from_fuse(id);
        }
    }
}
