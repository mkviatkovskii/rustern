//! Directory listing handles (data-model.md "Directory listing handle").
//!
//! Kernel offsets: 1 = ".", 2 = "..", 3 + n = n-th entry of this handle's listing. An entry
//! at position `p` (0 = ".", 1 = "..", 2 + n = entry n) carries offset `p + 1`, the offset
//! the kernel sends to continue after it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use bytes::Bytes;
use dashmap::DashMap;
use fuser::Errno;
use rustern_client::{Client, ClientError};
use rustern_core::inode::TernInode;
use rustern_msgs::{NameHash, ReadDirResp, StatDirectoryResp};
use tokio::sync::Mutex;

/// Most open listing handles.
pub const MAX_HANDLES: usize = 65_536;

/// Source of directory pages ([`Client::read_dir`]; a fake in tests).
pub trait Pager: Send + Sync {
    /// One page of `dir` from `start`; `next_hash` 0 means exhausted.
    fn read_dir(
        &self,
        dir: TernInode,
        start: NameHash,
    ) -> impl Future<Output = Result<ReadDirResp, ClientError>> + Send;
}

impl Pager for Client {
    fn read_dir(
        &self,
        dir: TernInode,
        start: NameHash,
    ) -> impl Future<Output = Result<ReadDirResp, ClientError>> + Send {
        Client::read_dir(self, dir, start)
    }
}

/// The parent of `dir`: `StatDirectoryResp.Owner`, or `dir` itself for the root.
pub fn parent_of(dir: TernInode, st: &StatDirectoryResp) -> Result<TernInode, ClientError> {
    if dir == TernInode::ROOT {
        Ok(dir)
    } else {
        Ok(TernInode::try_from(st.owner)?)
    }
}

/// One listed name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Target inode.
    pub id: TernInode,
    /// Raw name bytes.
    pub name: Bytes,
}

/// What sits at a listing position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// `.` (the directory itself).
    Dot(TernInode),
    /// `..` (the parent).
    DotDot(TernInode),
    /// A directory entry.
    Entry(Entry),
}

/// State of one open directory.
#[derive(Debug)]
pub struct DirHandle {
    dir: TernInode,
    parent: TernInode,
    /// Hash to fetch after `buf`; `None` = exhausted.
    next_hash: Option<NameHash>,
    /// The current page.
    buf: Vec<Entry>,
    /// Listing index of `buf[0]`.
    buf_start: u64,
}

impl DirHandle {
    /// A handle positioned at the start of `dir`'s listing.
    pub fn new(dir: TernInode, parent: TernInode) -> Self {
        Self {
            dir,
            parent,
            next_hash: Some(NameHash(0)),
            buf: Vec::new(),
            buf_start: 0,
        }
    }

    /// Up to `max` items from kernel `offset`, each with the offset that follows it. Once an
    /// item is found, at most one page is fetched, so a call stops at a page boundary
    /// (several pages only when skipping forward). An empty result means the end.
    pub async fn read(
        &mut self,
        pager: &impl Pager,
        offset: u64,
        max: usize,
    ) -> Result<Vec<(u64, Item)>, ClientError> {
        let mut out = Vec::new();
        let mut pos = offset;
        let mut fetched = false;
        while out.len() < max {
            let item = match pos {
                0 => Item::Dot(self.dir),
                1 => Item::DotDot(self.parent),
                p => {
                    let n = p.saturating_sub(2);
                    match self.entry(n) {
                        Some(e) => Item::Entry(e.clone()),
                        None if fetched && !out.is_empty() => break,
                        None => {
                            if !self.fetch_towards(pager, n).await? {
                                break;
                            }
                            fetched = true;
                            continue;
                        }
                    }
                }
            };
            pos = pos.saturating_add(1);
            out.push((pos, item));
        }
        Ok(out)
    }

    fn entry(&self, n: u64) -> Option<&Entry> {
        let i = usize::try_from(n.checked_sub(self.buf_start)?).ok()?;
        self.buf.get(i)
    }

    /// Moves the buffer towards entry `n`: re-lists from the start when `n` was passed,
    /// otherwise fetches the next page. Returns false at the end of the listing.
    async fn fetch_towards(&mut self, pager: &impl Pager, n: u64) -> Result<bool, ClientError> {
        if n < self.buf_start {
            self.next_hash = Some(NameHash(0));
            self.buf.clear();
            self.buf_start = 0;
        }
        let Some(start) = self.next_hash else {
            return Ok(false);
        };
        let page = pager.read_dir(self.dir, start).await?;
        self.buf_start = self.buf_start.saturating_add(self.buf.len() as u64);
        self.buf = page
            .results
            .into_iter()
            .map(|e| {
                Ok(Entry {
                    id: TernInode::try_from(e.target_id)?,
                    name: e.name,
                })
            })
            .collect::<Result<_, ClientError>>()?;
        self.next_hash = (page.next_hash.0 != 0).then_some(page.next_hash);
        Ok(true)
    }
}

/// Open listing handles, at most [`MAX_HANDLES`].
#[derive(Debug, Default)]
pub struct DirHandles {
    map: DashMap<u64, Arc<Mutex<DirHandle>>>,
    next: AtomicU64,
    count: AtomicUsize,
}

impl DirHandles {
    /// Stores `h`; EMFILE when [`MAX_HANDLES`] are open.
    pub fn insert(&self, h: DirHandle) -> Result<u64, Errno> {
        self.count
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_HANDLES).then(|| n.saturating_add(1))
            })
            .map_err(|_| Errno::EMFILE)?;
        let fh = self.next.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        self.map.insert(fh, Arc::new(Mutex::new(h)));
        Ok(fh)
    }

    /// The handle `fh`.
    pub fn get(&self, fh: u64) -> Option<Arc<Mutex<DirHandle>>> {
        self.map.get(&fh).map(|h| Arc::clone(&h))
    }

    /// Drops handle `fh`.
    pub fn remove(&self, fh: u64) {
        if self.map.remove(&fh).is_some() {
            self.count.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// Open handles.
    pub fn len(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }

    /// No handles open.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code
mod tests {
    use std::sync::Mutex as StdMutex;

    use rustern_msgs::{CurrentEdge, InodeId, TernTime};

    use super::*;

    const DIR: u64 = (1 << 61) | (7 << 8) | 2;
    const PARENT: u64 = (1 << 61) | (8 << 8) | 2;

    fn file(n: u64) -> u64 {
        (2 << 61) | ((100 + n) << 8) | 2
    }

    /// 25 entries in pages of 10: hash 0 → e0..e9, 100 → e10..e19, 200 → e20..e24.
    #[derive(Default)]
    struct FakePager {
        calls: StdMutex<Vec<u64>>,
    }

    impl Pager for FakePager {
        async fn read_dir(
            &self,
            dir: TernInode,
            start: NameHash,
        ) -> Result<ReadDirResp, ClientError> {
            assert_eq!(dir.id(), InodeId(DIR));
            self.calls.lock().unwrap().push(start.0);
            let (range, next) = match start.0 {
                0 => (0..10, 100),
                100 => (10..20, 200),
                200 => (20..25, 0),
                h => panic!("unexpected hash {h}"),
            };
            Ok(ReadDirResp {
                next_hash: NameHash(next),
                results: range
                    .map(|n| CurrentEdge {
                        target_id: InodeId(file(n)),
                        name_hash: NameHash(n),
                        name: Bytes::from(format!("e{n}")),
                        creation_time: TernTime(0),
                    })
                    .collect(),
            })
        }
    }

    fn handle() -> DirHandle {
        DirHandle::new(
            TernInode::new(DIR).unwrap(),
            TernInode::new(PARENT).unwrap(),
        )
    }

    fn name(i: &Item) -> String {
        match i {
            Item::Dot(_) => ".".into(),
            Item::DotDot(_) => "..".into(),
            Item::Entry(e) => String::from_utf8(e.name.to_vec()).unwrap(),
        }
    }

    /// Reads everything from `offset` as the kernel does, continuing at the last offset.
    async fn read_all(h: &mut DirHandle, p: &FakePager, mut offset: u64) -> Vec<(u64, String)> {
        let mut out = Vec::new();
        loop {
            let batch = h.read(p, offset, 7).await.unwrap();
            let Some(last) = batch.last() else { break };
            offset = last.0;
            out.extend(batch.iter().map(|(o, i)| (*o, name(i))));
        }
        out
    }

    fn expected(from_pos: u64) -> Vec<(u64, String)> {
        let mut all = vec![(1, ".".to_owned()), (2, "..".to_owned())];
        all.extend((0..25).map(|n| (n + 3, format!("e{n}"))));
        all.into_iter().skip(from_pos as usize).collect()
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
    }

    #[test]
    fn offsets_and_paging() {
        rt().block_on(async {
            let p = FakePager::default();
            let mut h = handle();
            let first = h.read(&p, 0, 100).await.unwrap();
            assert_eq!(first[0], (1, Item::Dot(TernInode::new(DIR).unwrap())));
            assert_eq!(first[1], (2, Item::DotDot(TernInode::new(PARENT).unwrap())));
            assert_eq!(first.len(), 12, "stops at the page boundary");
            let rest = read_all(&mut h, &p, 12).await;
            let mut all: Vec<(u64, String)> = first.iter().map(|(o, i)| (*o, name(i))).collect();
            all.extend(rest);
            assert_eq!(all, expected(0));
            assert_eq!(
                *p.calls.lock().unwrap(),
                [0, 100, 200],
                "follows next_hash to None"
            );
            // Exhausted: no further request.
            assert!(h.read(&p, 27, 10).await.unwrap().is_empty());
            assert_eq!(p.calls.lock().unwrap().len(), 3);
        });
    }

    #[test]
    fn seek_back_relists_and_skips() {
        rt().block_on(async {
            let p = FakePager::default();
            let mut h = handle();
            assert_eq!(read_all(&mut h, &p, 0).await, expected(0));
            // seekdir to offset 5 (after e2): re-lists from the start.
            assert_eq!(read_all(&mut h, &p, 5).await, expected(5));
            assert_eq!(*p.calls.lock().unwrap(), [0, 100, 200, 0, 100, 200]);
            // Within the current page: no request.
            let mut h = handle();
            let p = FakePager::default();
            let _ = h.read(&p, 2, 3).await.unwrap();
            let again = h.read(&p, 3, 2).await.unwrap();
            assert_eq!(name(&again[0].1), "e1");
            assert_eq!(*p.calls.lock().unwrap(), [0]);
        });
    }

    #[test]
    fn seek_forward_skips_pages() {
        rt().block_on(async {
            let p = FakePager::default();
            let mut h = handle();
            assert_eq!(read_all(&mut h, &p, 15).await, expected(15));
            assert_eq!(*p.calls.lock().unwrap(), [0, 100, 200]);
            assert!(h.read(&p, 1000, 5).await.unwrap().is_empty());
        });
    }

    #[test]
    fn parent_from_owner_or_self_for_root() {
        let st = StatDirectoryResp {
            owner: InodeId(PARENT),
            ..StatDirectoryResp::default()
        };
        let dir = TernInode::new(DIR).unwrap();
        assert_eq!(parent_of(dir, &st).unwrap().id(), InodeId(PARENT));
        let root = StatDirectoryResp::default();
        assert_eq!(parent_of(TernInode::ROOT, &root).unwrap(), TernInode::ROOT);
        assert!(parent_of(dir, &root).is_err(), "owner 0 is not an inode");
    }

    #[test]
    fn handle_table_is_bounded() {
        let t = DirHandles::default();
        assert!(t.is_empty());
        let mut last = 0;
        for _ in 0..MAX_HANDLES {
            last = t.insert(handle()).unwrap();
        }
        assert_eq!(t.insert(handle()), Err(Errno::EMFILE));
        t.remove(last);
        t.remove(last); // twice is harmless
        assert_eq!(t.len(), MAX_HANDLES - 1);
        let fh = t.insert(handle()).unwrap();
        assert!(t.get(fh).is_some());
        assert!(t.get(last).is_none());
    }
}
