//! Namespace requests: lookup, stat and one page of a directory listing.
//!
//! Concurrent `stat_file` / `stat_directory` calls on one inode share a single request
//! (Principle VI) through a `moka` cache whose entry is removed as soon as the load
//! completes, so a later call always sends a new request: nothing is cached (R6).

use bytes::Bytes;
use moka::future::Cache;
use rustern_core::inode::TernInode;
use rustern_msgs::{
    InodeId, LookupReq, LookupResp, NameHash, ReadDirReq, ReadDirResp, StatDirectoryReq,
    StatDirectoryResp, StatFileReq, StatFileResp,
};

use crate::{Client, ClientError};

/// In-flight stats, keyed by inode.
#[derive(Debug)]
pub(crate) struct InFlight {
    files: Cache<InodeId, StatFileResp>,
    dirs: Cache<InodeId, StatDirectoryResp>,
}

impl InFlight {
    pub(crate) fn new() -> Self {
        // Entries live only between a load completing and its invalidation.
        const CAPACITY: u64 = 1 << 16;
        Self {
            files: Cache::new(CAPACITY),
            dirs: Cache::new(CAPACITY),
        }
    }
}

/// Runs `load` once for all concurrent callers with the same `key`, caching nothing.
async fn shared<V, F>(cache: &Cache<InodeId, V>, key: InodeId, load: F) -> Result<V, ClientError>
where
    V: Clone + Send + Sync + 'static,
    F: Future<Output = Result<V, ClientError>>,
{
    let r = cache.try_get_with(key, load).await;
    cache.invalidate(&key).await;
    r.map_err(ClientError::shared)
}

impl Client {
    /// Resolves `name` in directory `dir`.
    pub async fn lookup(&self, dir: TernInode, name: &[u8]) -> Result<LookupResp, ClientError> {
        let req = LookupReq {
            dir_id: dir.id(),
            name: Bytes::copy_from_slice(name),
        };
        self.shard_request(dir.shard(), &req).await
    }

    /// Size and times of a file or symlink.
    pub async fn stat_file(&self, id: TernInode) -> Result<StatFileResp, ClientError> {
        let req = StatFileReq { id: id.id() };
        shared(
            &self.inner.in_flight.files,
            id.id(),
            self.shard_request(id.shard(), &req),
        )
        .await
    }

    /// Mtime, owner and info of a directory.
    pub async fn stat_directory(&self, id: TernInode) -> Result<StatDirectoryResp, ClientError> {
        let req = StatDirectoryReq { id: id.id() };
        shared(
            &self.inner.in_flight.dirs,
            id.id(),
            self.shard_request(id.shard(), &req),
        )
        .await
    }

    /// One page of `dir`'s entries from `start` (`NameHash(0)` for the first page). The
    /// response's `next_hash` is 0 once the listing is exhausted; the caller pages.
    pub async fn read_dir(
        &self,
        dir: TernInode,
        start: NameHash,
    ) -> Result<ReadDirResp, ClientError> {
        let req = ReadDirReq {
            dir_id: dir.id(),
            start_hash: start,
            mtu: self.inner.config.mtu,
        };
        self.shard_request(dir.shard(), &req).await
    }
}
