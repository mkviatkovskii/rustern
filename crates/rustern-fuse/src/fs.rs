//! [`RusternFs`]: the read-only TernFS filesystem served through the bridge
//! (contracts/filesystem.md).

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use fuser::{Errno, FileAttr, FileHandle, FopenFlags, Generation, INodeNo, OpenAccMode, OpenFlags};
use futures_util::{StreamExt, stream};
use rustern_client::{Client, ClientError, ReadBuf};
use rustern_core::inode::{InodeKind, TernInode};
use rustern_msgs::ErrorCode;
use rustix::fs::OFlags;

use crate::attr::{self, AttrConfig};
use crate::bridge::{
    AsyncFilesystem, DirEntry, DirEntryPlus, FsError, FsResult, GetAttrResponse, LookupResponse,
    OpenResponse, StatfsResponse,
};
use crate::dirhandle::{DirHandle, DirHandles, Item, parent_of};

/// Longest name TernFS stores.
const NAME_MAX: usize = 255;

/// The filesystem.
#[derive(Debug)]
pub struct RusternFs {
    client: Client,
    attrs: AttrConfig,
    dirs: DirHandles,
    files: DashMap<u64, Arc<rustern_client::FileHandle>>,
    next_fh: AtomicU64,
    /// Entries per readdir batch, and stats in flight per readdirplus.
    batch: usize,
}

impl RusternFs {
    /// Serves `client`'s cluster with attributes shaped by `attrs`.
    pub fn new(client: Client, attrs: AttrConfig) -> Self {
        let batch = usize::try_from(client.config().readdir_batch).unwrap_or(usize::MAX);
        Self {
            client,
            attrs,
            dirs: DirHandles::default(),
            files: DashMap::new(),
            next_fh: AtomicU64::new(0),
            batch,
        }
    }

    /// Attributes of `id` and their TTL.
    async fn stat(&self, id: TernInode) -> FsResult<(FileAttr, Duration)> {
        Ok(match id.kind() {
            InodeKind::Directory => self.attrs.dir(id, &self.client.stat_directory(id).await?),
            InodeKind::File | InodeKind::Symlink => {
                self.attrs.file(id, &self.client.stat_file(id).await?)
            }
        })
    }

    fn entry(&self, attr: FileAttr) -> LookupResponse {
        LookupResponse {
            ttl: self.attrs.entry_ttl,
            attr,
            generation: Generation(0),
        }
    }

    async fn dir_items(&self, fh: FileHandle, offset: u64) -> FsResult<Vec<(u64, Item)>> {
        let h = self.dirs.get(fh.0).ok_or(Errno::EBADF)?;
        let mut h = h.lock().await;
        Ok(h.read(&self.client, offset, self.batch).await?)
    }
}

fn inode(ino: INodeNo) -> FsResult<TernInode> {
    TernInode::from_fuse(ino.0).map_err(|e| ClientError::from(e).into())
}

fn is_not_found(e: &FsError) -> bool {
    e.errno == Errno::ENOENT
}

fn item_parts(item: &Item) -> (TernInode, OsString) {
    match item {
        Item::Dot(id) => (*id, ".".into()),
        Item::DotDot(id) => (*id, "..".into()),
        Item::Entry(e) => (e.id, OsString::from_vec(e.name.to_vec())),
    }
}

impl AsyncFilesystem for RusternFs {
    type Buf = ReadBuf;
    const READDIRPLUS: bool = true;

    async fn lookup(&self, parent: INodeNo, name: &OsStr) -> FsResult<LookupResponse> {
        let dir = inode(parent)?;
        if dir.kind() != InodeKind::Directory {
            return Err(Errno::ENOTDIR.into());
        }
        if name.len() > NAME_MAX {
            return Err(Errno::ENAMETOOLONG.into());
        }
        match self.client.lookup(dir, name.as_bytes()).await {
            Ok(r) => {
                let id = TernInode::try_from(r.target_id).map_err(ClientError::from)?;
                let (attr, _) = self.stat(id).await?;
                Ok(self.entry(attr))
            }
            // A negative entry, cached by the kernel for the entry TTL.
            Err(ClientError::Tern {
                code: ErrorCode::NAME_NOT_FOUND,
                ..
            }) => Ok(self.entry(self.attrs.negative())),
            Err(e) => Err(e.into()),
        }
    }

    async fn getattr(&self, ino: INodeNo, _fh: Option<FileHandle>) -> FsResult<GetAttrResponse> {
        let (attr, ttl) = self.stat(inode(ino)?).await?;
        Ok(GetAttrResponse { ttl, attr })
    }

    async fn readlink(&self, ino: INodeNo) -> FsResult<Vec<u8>> {
        let id = inode(ino)?;
        if id.kind() != InodeKind::Symlink {
            return Err(Errno::EINVAL.into());
        }
        Ok(self.client.read_link(id).await?)
    }

    async fn open(&self, ino: INodeNo, flags: OpenFlags) -> FsResult<OpenResponse> {
        let id = inode(ino)?;
        let o = OFlags::from_bits_retain(flags.0.cast_unsigned());
        if flags.acc_mode() != OpenAccMode::O_RDONLY || o.intersects(OFlags::TRUNC | OFlags::CREATE)
        {
            return Err(Errno::EROFS.into());
        }
        if id.kind() == InodeKind::Directory {
            return Err(Errno::EISDIR.into());
        }
        let h = self.client.open_file(id).await?;
        let fh = self.next_fh.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        self.files.insert(fh, Arc::new(h));
        // Files are immutable: keep the page cache across opens (FR-011); no set-time
        // request is sent (DEV-002).
        Ok(OpenResponse {
            fh: FileHandle(fh),
            flags: FopenFlags::FOPEN_KEEP_CACHE,
        })
    }

    async fn read(
        &self,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
    ) -> FsResult<ReadBuf> {
        let h = self
            .files
            .get(&fh.0)
            .map(|h| Arc::clone(&h))
            .ok_or(Errno::EBADF)?;
        Ok(h.read(offset, size).await?)
    }

    async fn release(&self, _ino: INodeNo, fh: FileHandle) -> FsResult<()> {
        self.files.remove(&fh.0);
        Ok(())
    }

    async fn opendir(&self, ino: INodeNo, _flags: OpenFlags) -> FsResult<OpenResponse> {
        let dir = inode(ino)?;
        if dir.kind() != InodeKind::Directory {
            return Err(Errno::ENOTDIR.into());
        }
        let st = self.client.stat_directory(dir).await?;
        let parent = parent_of(dir, &st)?;
        let fh = self.dirs.insert(DirHandle::new(dir, parent))?;
        Ok(OpenResponse {
            fh: FileHandle(fh),
            // The kernel serves relistings from its cache (ternfuse sets cache-dir too) until
            // the directory mtime changes, seen within the directory attribute TTL (DEV-003).
            // Without keep-cache, Linux 6.18 drops the cached listing on every opendir.
            flags: FopenFlags::FOPEN_CACHE_DIR.union(FopenFlags::FOPEN_KEEP_CACHE),
        })
    }

    async fn readdir(&self, _ino: INodeNo, fh: FileHandle, offset: u64) -> FsResult<Vec<DirEntry>> {
        let items = self.dir_items(fh, offset).await?;
        Ok(items
            .iter()
            .map(|(offset, item)| {
                let (id, name) = item_parts(item);
                DirEntry {
                    ino: INodeNo(id.to_fuse()),
                    offset: *offset,
                    kind: attr::kind(id.kind()),
                    name,
                }
            })
            .collect())
    }

    async fn readdirplus(
        &self,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
    ) -> FsResult<Vec<DirEntryPlus>> {
        let items = self.dir_items(fh, offset).await?;
        // Every entry stat-ed concurrently, at most `batch` in flight, in listing order.
        let ids: Vec<TernInode> = items.iter().map(|(_, item)| item_parts(item).0).collect();
        let stats: Vec<_> = stream::iter(ids)
            .map(|id| self.stat(id))
            .buffered(self.batch.max(1))
            .collect()
            .await;
        let mut out = Vec::with_capacity(items.len());
        for ((offset, item), st) in items.iter().zip(stats) {
            match st {
                Ok((attr, _)) => out.push(DirEntryPlus {
                    offset: *offset,
                    name: item_parts(item).1,
                    entry: self.entry(attr),
                }),
                // Removed since it was listed: leave it out.
                Err(e) if is_not_found(&e) && matches!(item, Item::Entry(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    async fn releasedir(&self, _ino: INodeNo, fh: FileHandle) -> FsResult<()> {
        self.dirs.remove(fh.0);
        Ok(())
    }

    async fn statfs(&self, _ino: INodeNo) -> FsResult<StatfsResponse> {
        // Read-only (ST_RDONLY) comes from the `ro` mount option.
        Ok(StatfsResponse {
            bsize: 4096,
            frsize: 4096,
            namelen: u32::try_from(NAME_MAX).unwrap_or(u32::MAX),
            ..StatfsResponse::default()
        })
    }
}
