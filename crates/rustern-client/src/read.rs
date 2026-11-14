//! File reads (US2): the span list of a file (cached, R6), then the pages covering a read
//! fetched from block services straight into one pooled buffer (R12). Memory is bounded by
//! a per-file count of semaphore permits, one per buffer, and globally by the buffer pool's
//! size (FR-012).
//!
//! A missing or corrupt block is EIO; there is no Reed-Solomon reconstruction (DEV-004).
//! Mirrored spans fail over across their readable copies.

use std::sync::{Arc, Weak};

use dashmap::DashMap;
use futures_util::future::try_join_all;
use rustern_core::crc::crc32c;
use rustern_core::inode::TernInode;
use rustern_core::layout::PAGE;
use rustern_core::span::{BlockRef, BlocksSpan, Span, SpanBody, can_read};
use rustern_msgs::{InodeId, LocalFileSpansReq, LocalFileSpansResp};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::Instrument;

use crate::blocks::{Conns, fetch_pages};
use crate::bufpool::{self, BufPool, PooledBuf};
use crate::cache::{SpanCache, span_at};
use crate::config::{ClientConfig, READ_BUF_SIZE, ReqTimeouts};
use crate::{Client, ClientError};

/// Largest read; the kernel is told so at mount (`max_read`).
pub const MAX_READ: u32 = if cfg!(feature = "read2m") {
    2 << 20
} else {
    1 << 20
};

const PAGE_BYTES: usize = 4096;

fn bounds() -> ClientError {
    ClientError::Malformed {
        what: "read out of bounds",
    }
}

fn closed<E>(_: E) -> ClientError {
    ClientError::Malformed {
        what: "read permits closed",
    }
}

fn to_usize(n: u64) -> Result<usize, ClientError> {
    usize::try_from(n).map_err(|_| bounds())
}

/// Read state shared by a client's open files.
#[derive(Debug)]
pub(crate) struct Reads {
    pool: BufPool,
    per_file: usize,
    files: DashMap<InodeId, Weak<Semaphore>>,
    spans: SpanCache,
}

impl Reads {
    pub(crate) fn new(c: &ClientConfig) -> Result<Self, ClientError> {
        let buffers = (c.memory_limit / READ_BUF_SIZE).max(1);
        Ok(Self {
            pool: bufpool::new(buffers)?,
            per_file: (c.file_memory_limit / READ_BUF_SIZE).clamp(1, buffers),
            files: DashMap::new(),
            spans: SpanCache::default(),
        })
    }
}

/// A block span as fault tests see it: offset, mirrored, `(block id, service id)`s.
#[doc(hidden)]
pub type BlockLayout = (u64, bool, Vec<(u64, u64)>);

/// An open file. Reads of one file share its permits; the handle is cheap to keep open.
#[derive(Debug)]
pub struct FileHandle {
    client: Client,
    id: InodeId,
    spans: Arc<[Span]>,
    permits: Arc<Semaphore>,
}

impl Drop for FileHandle {
    fn drop(&mut self) {
        let ours = Arc::downgrade(&self.permits);
        self.client.inner.reads.files.remove_if(&self.id, |_, w| {
            Weak::ptr_eq(w, &ours) && w.strong_count() == 1
        });
    }
}

impl FileHandle {
    /// File size: the end of the last span.
    pub fn size(&self) -> u64 {
        self.spans.last().map_or(0, Span::end)
    }

    /// Start offsets of the file's spans (tests bias reads to cross them).
    #[doc(hidden)]
    pub fn span_offsets(&self) -> Vec<u64> {
        self.spans.iter().map(|s| s.byte_offset).collect()
    }

    /// For each block span: its offset, whether it is mirrored, and its blocks as
    /// `(block id, block service id)` (fault tests corrupt or stop them).
    #[doc(hidden)]
    pub fn block_layout(&self) -> Vec<BlockLayout> {
        self.spans
            .iter()
            .filter_map(|s| match &s.body {
                SpanBody::Blocks(b) => Some((
                    s.byte_offset,
                    b.is_mirrored(),
                    b.blocks
                        .iter()
                        .map(|r| (r.block_id.0, r.service.id.0))
                        .collect(),
                )),
                SpanBody::Inline { .. } => None,
            })
            .collect()
    }

    /// Up to `size` bytes from `offset`; fewer at the end of the file, none past it.
    pub async fn read(&self, offset: u64, size: u32) -> Result<ReadBuf, ClientError> {
        if size > MAX_READ {
            return Err(ClientError::ReadTooLarge(size));
        }
        let end = offset.saturating_add(u64::from(size)).min(self.size());
        if offset >= end {
            return Ok(ReadBuf::default());
        }
        let reads = &self.client.inner.reads;
        let file = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(closed)?;
        let mut buf = reads.pool.get().await.map_err(closed)?;
        let fetch = Blocks(&self.client.inner.config.block, &self.client.inner.conns);
        let start = assemble(&self.spans, offset, end, &mut buf, &fetch)
            .instrument(tracing::warn_span!("read", inode = self.id.0, offset))
            .await?;
        Ok(ReadBuf {
            buf: Some(buf),
            start,
            len: to_usize(end.checked_sub(offset).ok_or_else(bounds)?)?,
            _file: Some(file),
        })
    }
}

/// Bytes of one read; holds its buffer and permits until dropped.
#[derive(Debug, Default)]
pub struct ReadBuf {
    // Field order matters: the buffer goes back to the pool before the file's permit is
    // released.
    buf: Option<PooledBuf>,
    start: usize,
    len: usize,
    _file: Option<OwnedSemaphorePermit>,
}

impl AsRef<[u8]> for ReadBuf {
    fn as_ref(&self) -> &[u8] {
        self.buf
            .as_deref()
            .and_then(|b| b.get(self.start..self.start.saturating_add(self.len)))
            .unwrap_or(&[])
    }
}

impl Client {
    /// Opens a file or symlink for reading: loads (or reuses) its span list.
    pub async fn open_file(&self, inode: TernInode) -> Result<FileHandle, ClientError> {
        let reads = &self.inner.reads;
        let spans = reads
            .spans
            .get_or_load(inode.id(), self.file_spans(inode))
            .await?;
        let permits = {
            let mut e = reads.files.entry(inode.id()).or_default();
            if let Some(p) = e.upgrade() {
                p
            } else {
                let p = Arc::new(Semaphore::new(reads.per_file));
                *e = Arc::downgrade(&p);
                p
            }
        };
        Ok(FileHandle {
            client: self.clone(),
            id: inode.id(),
            spans,
            permits,
        })
    }

    /// The target of a symlink (its whole body, at most [`MAX_READ`]).
    pub async fn read_link(&self, inode: TernInode) -> Result<Vec<u8>, ClientError> {
        let f = self.open_file(inode).await?;
        let size = u32::try_from(f.size())
            .ok()
            .filter(|s| *s <= MAX_READ)
            .ok_or(ClientError::ReadTooLarge(u32::MAX))?;
        Ok(f.read(0, size).await?.as_ref().to_vec())
    }

    /// Every span of a file, paging `LocalFileSpans` until `next_offset == 0`.
    async fn file_spans(&self, inode: TernInode) -> Result<Vec<Span>, ClientError> {
        let mut out: Vec<Span> = Vec::new();
        let mut offset = 0;
        loop {
            let req = LocalFileSpansReq {
                file_id: inode.id(),
                byte_offset: offset,
                limit: 0,
                mtu: self.inner.config.mtu,
            };
            let LocalFileSpansResp {
                next_offset,
                block_services,
                spans,
            } = self.shard_request(inode.shard(), &req).await?;
            for f in spans {
                let s = Span::try_from((f, block_services.as_slice()))?;
                // A page may repeat the span the previous one ended with.
                if s.byte_offset >= out.last().map_or(0, Span::end) && s.size > 0 {
                    out.push(s);
                }
            }
            if next_offset == 0 {
                return Ok(out);
            }
            if next_offset <= offset {
                return Err(ClientError::Malformed {
                    what: "file spans did not advance",
                });
            }
            offset = next_offset;
        }
    }
}

/// Fetches block pages; a trait so reads can be tested without block services.
pub(crate) trait Fetch {
    async fn fetch(
        &self,
        block: &BlockRef,
        page_start: u32,
        pages: &mut [&mut [u8]],
    ) -> Result<(), ClientError>;
}

struct Blocks<'a>(&'a ReqTimeouts, &'a Conns);

impl Fetch for Blocks<'_> {
    async fn fetch(
        &self,
        block: &BlockRef,
        page_start: u32,
        pages: &mut [&mut [u8]],
    ) -> Result<(), ClientError> {
        fetch_pages(
            self.1,
            &block.service,
            block.block_id,
            page_start,
            pages,
            self.0,
        )
        .await
    }
}

/// Fills `buf` with file bytes `[offset, end)` starting at the returned index (the
/// in-page offset of `offset` when the first span is a block span, so block pages land
/// page-aligned in the buffer). Spans are filled in order; a block span's last page may
/// spill past its end and is overwritten by the next span.
pub(crate) async fn assemble<F: Fetch>(
    spans: &[Span],
    offset: u64,
    end: u64,
    buf: &mut [u8],
    f: &F,
) -> Result<usize, ClientError> {
    let first = span_at(spans, offset).ok_or(ClientError::NoSpan(offset))?;
    let lead = match first.body {
        SpanBody::Blocks(_) => offset
            .checked_sub(first.byte_offset)
            .and_then(|o| o.checked_rem(PAGE))
            .ok_or_else(bounds)?,
        SpanBody::Inline { .. } => 0,
    };
    let mut pos = offset;
    while pos < end {
        let s = span_at(spans, pos).ok_or(ClientError::NoSpan(pos))?;
        let from = pos.checked_sub(s.byte_offset).ok_or_else(bounds)?;
        let to = end
            .min(s.end())
            .checked_sub(s.byte_offset)
            .ok_or_else(bounds)?;
        let n = to_usize(to.checked_sub(from).ok_or_else(bounds)?)?;
        let at = to_usize(
            lead.checked_add(pos.checked_sub(offset).ok_or_else(bounds)?)
                .ok_or_else(bounds)?,
        )?;
        let out_end = at.checked_add(n).ok_or_else(bounds)?;
        match &s.body {
            SpanBody::Inline { data } => {
                let stored = data.get(to_usize(from)?..).unwrap_or(&[]);
                let copy = stored.len().min(n);
                let dst = buf.get_mut(at..out_end).ok_or_else(bounds)?;
                let (head, tail) = dst.split_at_mut(copy);
                head.copy_from_slice(stored.get(..copy).ok_or_else(bounds)?);
                tail.fill(0);
            }
            SpanBody::Blocks(b) => {
                let region_start = at
                    .checked_sub(to_usize(from.checked_rem(PAGE).ok_or_else(bounds)?)?)
                    .ok_or_else(bounds)?;
                let region = buf.get_mut(region_start..).ok_or_else(bounds)?;
                read_blocks(b, from, to, region, f).await?;
                let cap = b.geometry().capacity().ok_or_else(bounds)?;
                if to > cap {
                    let zero_from = at
                        .checked_add(to_usize(cap.saturating_sub(from))?)
                        .ok_or_else(bounds)?;
                    buf.get_mut(zero_from..out_end).ok_or_else(bounds)?.fill(0);
                }
                if from == 0 && to == u64::from(s.size) {
                    let got = crc32c(buf.get(at..out_end).ok_or_else(bounds)?);
                    if got != s.crc.0 {
                        tracing::error!(alert = true, span = s.byte_offset, "span crc mismatch");
                        return Err(ClientError::Malformed {
                            what: "span crc mismatch",
                        });
                    }
                }
            }
        }
        pos = s.byte_offset.checked_add(to).ok_or_else(bounds)?;
    }
    to_usize(lead)
}

/// Fetches the stored pages of span bytes `[from, to)` into `region`, which starts at the
/// page containing `from`. Blocks are fetched concurrently.
async fn read_blocks<F: Fetch>(
    b: &BlocksSpan,
    from: u64,
    to: u64,
    region: &mut [u8],
    f: &F,
) -> Result<(), ClientError> {
    let g = b.geometry();
    let (first, end) = g.span_pages(from, to).ok_or_else(bounds)?;
    if first >= end {
        return Ok(());
    }
    let ranges = g.ranges(from, to).ok_or_else(bounds)?;
    let len = to_usize(end.checked_sub(first).ok_or_else(bounds)?)?
        .checked_mul(PAGE_BYTES)
        .ok_or_else(bounds)?;
    let region = region.get_mut(..len).ok_or_else(bounds)?;
    let mut per_block: Vec<Vec<&mut [u8]>> = (0..g.data_blocks).map(|_| Vec::new()).collect();
    for (p, chunk) in (first..end).zip(region.chunks_mut(PAGE_BYTES)) {
        let (block, _) = g.page(p).ok_or_else(bounds)?;
        per_block
            .get_mut(to_usize(block)?)
            .ok_or_else(bounds)?
            .push(chunk);
    }
    let mut jobs = Vec::with_capacity(ranges.len());
    for (r, pages) in ranges
        .iter()
        .zip(per_block.into_iter().filter(|p| !p.is_empty()))
    {
        let page_start = u32::try_from(r.first_page).map_err(|_| bounds())?;
        jobs.push(fetch_block(b, to_usize(r.block)?, page_start, pages, f));
    }
    try_join_all(jobs).await?;
    Ok(())
}

async fn fetch_block<F: Fetch>(
    b: &BlocksSpan,
    block: usize,
    page_start: u32,
    mut pages: Vec<&mut [u8]>,
    f: &F,
) -> Result<(), ClientError> {
    if !b.is_mirrored() {
        let r = b.blocks.get(block).ok_or_else(bounds)?;
        if !can_read(r.service.flags) {
            return Err(ClientError::Unreadable {
                block_id: r.block_id,
            });
        }
        return f.fetch(r, page_start, &mut pages).await.inspect_err(|e| {
            tracing::error!(
                alert = true,
                block_id = r.block_id.0,
                service = r.service.id.0,
                page = page_start,
                error = %e,
                "block read failed; no reconstruction (DEV-004)"
            );
        });
    }
    let mut last = None;
    for r in b.blocks.iter().filter(|r| can_read(r.service.flags)) {
        match f.fetch(r, page_start, &mut pages).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::warn!(
                    block_id = r.block_id.0,
                    service = r.service.id.0,
                    page = page_start,
                    error = %e,
                    "mirrored copy failed; trying the next copy"
                );
                last = Some(e);
            }
        }
    }
    let e = last.unwrap_or_else(|| ClientError::Unreadable {
        block_id: b.blocks.first().map(|r| r.block_id).unwrap_or_default(),
    });
    tracing::error!(
        alert = true,
        block_id = b.blocks.first().map_or(0, |r| r.block_id.0),
        page = page_start,
        error = %e,
        "no readable copy of a mirrored span"
    );
    Err(e)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    use bytes::Bytes;
    use proptest::prelude::*;
    use rustern_core::layout::Geometry;
    use rustern_msgs::{BlockId, BlockService, BlockServiceFlags, Crc};

    use super::*;

    /// Blocks in memory; `failing` ones return an I/O error.
    #[derive(Default)]
    struct Store {
        blocks: HashMap<u64, Vec<u8>>,
        failing: HashSet<u64>,
        calls: Mutex<Vec<(u64, u32, usize)>>,
    }

    impl Fetch for Store {
        async fn fetch(
            &self,
            block: &BlockRef,
            page_start: u32,
            pages: &mut [&mut [u8]],
        ) -> Result<(), ClientError> {
            let id = block.block_id.0;
            self.calls
                .lock()
                .unwrap()
                .push((id, page_start, pages.len()));
            if self.failing.contains(&id) {
                return Err(ClientError::Io(std::io::Error::other("down")));
            }
            let data = &self.blocks[&id];
            for (i, p) in pages.iter_mut().enumerate() {
                let o = (page_start as usize + i) * PAGE_BYTES;
                p.copy_from_slice(&data[o..o + PAGE_BYTES]);
            }
            Ok(())
        }
    }

    fn service(readable: bool) -> BlockService {
        BlockService {
            flags: BlockServiceFlags(if readable { 0 } else { 0x2 }),
            ..BlockService::default()
        }
    }

    /// A file of spans plus its logical bytes.
    struct File {
        spans: Vec<Span>,
        bytes: Vec<u8>,
        store: Store,
        next_block: u64,
    }

    impl File {
        fn new() -> Self {
            Self {
                spans: Vec::new(),
                bytes: Vec::new(),
                store: Store::default(),
                next_block: 1,
            }
        }

        fn push(&mut self, size: u32, body: SpanBody, logical: &[u8]) {
            self.spans.push(Span {
                byte_offset: self.bytes.len() as u64,
                size,
                crc: Crc(crc32c(logical)),
                body,
            });
            self.bytes.extend_from_slice(logical);
        }

        fn inline(&mut self, data: &[u8], size: u32) {
            let mut logical = data.to_vec();
            logical.resize(size as usize, 0);
            self.push(
                size,
                SpanBody::Inline {
                    data: Bytes::copy_from_slice(data),
                },
                &logical,
            );
        }

        /// A block span; `copies` > 1 makes it mirrored (requires `d == 1`).
        fn blocks(
            &mut self,
            d: u8,
            cell_pages: u32,
            stripes: u8,
            size: u32,
            copies: &[bool],
            seed: u8,
        ) {
            let g = Geometry {
                data_blocks: u64::from(d),
                cell_size: u64::from(cell_pages) * PAGE,
                stripes: u64::from(stripes),
            };
            let cap = g.capacity().unwrap();
            let logical: Vec<u8> = (0..u64::from(size))
                .map(|o| {
                    if o < cap {
                        (o as u8).wrapping_mul(31).wrapping_add(seed) | 1
                    } else {
                        0
                    }
                })
                .collect();
            let mut per_block = vec![vec![0u8; g.block_size().unwrap() as usize]; d as usize];
            for o in 0..cap.min(u64::from(size)) {
                let (b, off) = g.locate(o).unwrap();
                per_block[b as usize][off as usize] = logical[o as usize];
            }
            let mut refs = Vec::new();
            if d == 1 {
                for &readable in copies {
                    let id = self.next_block;
                    self.next_block += 1;
                    self.store.blocks.insert(id, per_block[0].clone());
                    refs.push(BlockRef {
                        service: service(readable),
                        block_id: BlockId(id),
                        crc: Crc(0),
                    });
                }
            } else {
                for data in per_block {
                    let id = self.next_block;
                    self.next_block += 1;
                    self.store.blocks.insert(id, data);
                    refs.push(BlockRef {
                        service: service(true),
                        block_id: BlockId(id),
                        crc: Crc(0),
                    });
                }
            }
            let parity = u8::try_from(refs.len()).unwrap() - d;
            let body = SpanBody::Blocks(BlocksSpan {
                data_blocks: d,
                parity_blocks: parity,
                stripes,
                cell_size: cell_pages * 4096,
                blocks: refs,
                stripes_crc: vec![Crc(0); stripes as usize],
            });
            self.push(size, body, &logical);
        }

        async fn read(&self, offset: u64, len: u64) -> Result<Vec<u8>, ClientError> {
            let end = (offset + len).min(self.bytes.len() as u64);
            let mut buf = vec![0xEEu8; READ_BUF_SIZE];
            let start = assemble(&self.spans, offset, end, &mut buf, &self.store).await?;
            Ok(buf[start..start + (end - offset) as usize].to_vec())
        }
    }

    fn sample() -> File {
        let mut f = File::new();
        f.inline(b"hello", 9); // inline, zero-extended
        f.blocks(1, 2, 3, 6 * 4096, &[true, true], 1); // mirrored, exactly full
        f.blocks(3, 2, 2, 3 * 2 * 8192 + 5000, &[], 2); // RS-like, zero tail past capacity
        f.blocks(1, 1, 4, 3 * 4096 + 17, &[true], 3); // mirrored, partly filled last cell
        f.inline(b"", 3);
        f
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
    }

    #[test]
    fn whole_file_and_every_span_boundary() {
        let f = sample();
        rt().block_on(async {
            let n = f.bytes.len() as u64;
            let mut o = 0;
            while o < n {
                let got = f.read(o, u64::from(MAX_READ)).await.unwrap();
                let want = &f.bytes[o as usize..(o + u64::from(MAX_READ)).min(n) as usize];
                assert_eq!(got, want, "offset {o}");
                o += u64::from(MAX_READ);
            }
            for s in &f.spans {
                for (a, b) in [
                    (s.byte_offset, s.end()),
                    (s.byte_offset.saturating_sub(3), s.byte_offset + 2),
                ] {
                    let got = f.read(a, b - a).await.unwrap();
                    assert_eq!(got, &f.bytes[a as usize..b.min(n) as usize], "[{a}, {b})");
                }
            }
        });
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]
        #[test]
        fn random_reads_match(o in 0u64..80_000, len in 1u64..70_000) {
            let f = sample();
            let n = f.bytes.len() as u64;
            let o = o % n;
            let got = rt().block_on(f.read(o, len)).unwrap();
            prop_assert_eq!(got, &f.bytes[o as usize..(o + len).min(n) as usize]);
        }
    }

    #[test]
    fn mirror_fails_over_and_skips_unreadable() {
        let mut f = File::new();
        f.blocks(1, 1, 2, 8192, &[false, true, true], 9);
        f.store.failing.insert(2); // the first readable copy is down
        let got = rt().block_on(f.read(0, 8192)).unwrap();
        assert_eq!(got, f.bytes);
        let calls: Vec<u64> = f.store.calls.lock().unwrap().iter().map(|c| c.0).collect();
        assert_eq!(calls, [2, 3], "unreadable copy 1 never asked");

        f.store.failing.insert(3);
        assert!(matches!(
            rt().block_on(f.read(0, 1)),
            Err(ClientError::Io(_))
        ));
    }

    #[test]
    fn rs_block_on_unreadable_service_is_eio() {
        let mut f = File::new();
        f.blocks(2, 1, 1, 8192, &[], 4);
        if let SpanBody::Blocks(b) = &mut f.spans[0].body {
            b.blocks[1].service = service(false);
        }
        let r = rt().block_on(f.read(0, 8192));
        assert!(
            matches!(
                r,
                Err(ClientError::Unreadable {
                    block_id: BlockId(2)
                })
            ),
            "{r:?}"
        );
        // A read not touching block 1 succeeds (no reconstruction needed).
        assert_eq!(rt().block_on(f.read(0, 4096)).unwrap(), &f.bytes[..4096]);
    }

    #[test]
    fn one_fetch_per_block_page_aligned() {
        let mut f = File::new();
        f.blocks(3, 1, 2, 6 * 4096, &[], 5);
        rt().block_on(f.read(100, 5 * 4096)).unwrap();
        let mut calls = f.store.calls.lock().unwrap().clone();
        calls.sort_unstable();
        // Span pages 0..6: block 0 gets pages 0,3; block 1 gets 1,4; block 2 gets 2,5.
        assert_eq!(calls, [(1, 0, 2), (2, 0, 2), (3, 0, 2)]);
    }

    #[test]
    fn gap_and_corruption_are_errors() {
        let mut f = sample();
        f.spans.remove(1);
        assert!(matches!(
            rt().block_on(f.read(0, 20)),
            Err(ClientError::NoSpan(9))
        ));

        let mut f = File::new();
        f.blocks(1, 1, 1, 4096, &[true], 6);
        f.spans[0].crc = Crc(1);
        assert!(matches!(
            rt().block_on(f.read(0, 4096)),
            Err(ClientError::Malformed { .. })
        ));
    }
}
