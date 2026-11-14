//! Block fetches (`FETCH_BLOCK_WITH_CRC` over TCP, upstream `go/client/client.go`).
//!
//! The response is a header, then `pages` × (4096 data bytes, little-endian `u32`
//! CRC32C). [`Scatter`] lays the stream over the caller's page slices and a side array of
//! CRC words, so data lands where the reply is built with no copy (R12); every page is
//! then verified in place.

use std::io::{self, IoSliceMut};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Instant;

use dashmap::DashMap;
use rustern_msgs::frame::{BLOCKS_RESP_HEADER_LEN, blocks_resp_kind, encode_blocks_req};
use rustern_msgs::{
    AddrsInfo, BlockId, BlockService, BlocksMessageKind, Crc, ErrorCode, FetchBlockWithCrcReq,
    InodeId, TERN_PAGE_SIZE,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::config::ReqTimeouts;
use crate::{ClientError, Service, metrics, timeouts};

/// Data bytes per page.
pub const PAGE: usize = TERN_PAGE_SIZE as usize;
const CRC: usize = 4;
const PAGE_WITH_CRC: usize = PAGE + CRC;
/// Most slices handed to one `read_vectored` call.
const MAX_SLICES: usize = 128;

/// Position in a page stream of a known page count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scatter {
    pos: usize,
    len: usize,
}

impl Scatter {
    /// A stream of `pages` pages (from the request, never from the server).
    pub fn new(pages: usize) -> Self {
        Self {
            pos: 0,
            len: pages.saturating_mul(PAGE_WITH_CRC),
        }
    }

    /// Whole stream received.
    pub fn done(&self) -> bool {
        self.pos >= self.len
    }

    /// Records `n` bytes received into the slices last returned.
    pub fn advance(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n).min(self.len);
    }

    /// Slices for the bytes still missing: page data into `pages`, CRC words into `crcs`.
    pub fn next_slices<'a>(
        &self,
        pages: &'a mut [&mut [u8]],
        crcs: &'a mut [[u8; CRC]],
    ) -> Vec<IoSliceMut<'a>> {
        let mut out = Vec::with_capacity(MAX_SLICES);
        let first = self.pos / PAGE_WITH_CRC;
        let mut within = self.pos % PAGE_WITH_CRC;
        for (page, crc) in pages.iter_mut().zip(crcs.iter_mut()).skip(first) {
            if out.len().saturating_add(2) > MAX_SLICES {
                break;
            }
            if within < PAGE {
                if let Some(rest) = page.get_mut(within..PAGE) {
                    out.push(IoSliceMut::new(rest));
                }
                out.push(IoSliceMut::new(crc));
            } else if let Some(rest) = crc.get_mut(within.saturating_sub(PAGE)..) {
                out.push(IoSliceMut::new(rest));
            }
            within = 0;
        }
        out
    }
}

/// Checks every page against its CRC word.
pub fn verify(
    block_id: BlockId,
    pages: &[&mut [u8]],
    crcs: &[[u8; CRC]],
) -> Result<(), ClientError> {
    for (i, (page, crc)) in pages.iter().zip(crcs).enumerate() {
        if rustern_core::crc::crc32c(page) != u32::from_le_bytes(*crc) {
            return Err(ClientError::CrcMismatch {
                block_id,
                page: u32::try_from(i).unwrap_or(u32::MAX),
            });
        }
    }
    Ok(())
}

/// Fuzz entry: scatters `data` as a 4-page stream in chunks of its first byte's size,
/// then verifies. Must never panic.
#[doc(hidden)]
pub fn page_stream(data: &[u8]) -> Result<(), ClientError> {
    let (step, stream) = match data.split_first() {
        Some((s, rest)) => (usize::from(*s).max(1), rest),
        None => return Err(ClientError::Malformed { what: "empty" }),
    };
    let mut store = vec![0u8; 4 * PAGE];
    let mut pages: Vec<&mut [u8]> = store.chunks_mut(PAGE).collect();
    let mut crcs = [[0u8; CRC]; 4];
    let mut sc = Scatter::new(pages.len());
    let mut input = stream;
    while !sc.done() {
        if input.is_empty() {
            return Err(ClientError::Malformed {
                what: "short page stream",
            });
        }
        let (chunk, rest) = input.split_at(step.min(input.len()));
        input = rest;
        let mut slices = sc.next_slices(&mut pages, &mut crcs);
        let n = io::Read::read_vectored(&mut &*chunk, &mut slices)?;
        sc.advance(n);
    }
    verify(BlockId(0), &pages, &crcs)
}

/// Idle connections kept per block-service address.
pub const IDLE_PER_ADDR: usize = 64;

/// Idle block-service connections, reused across fetches. Upstream pipelines many requests
/// on one connection per service; here a connection carries one fetch at a time, so a small
/// read never queues behind a large one, and a connection returns only after a complete
/// response, so a cancelled or failed fetch never leaves a half-read stream behind. Last in,
/// first out, like [`crate::bufpool::BufPool`]: after a burst, a lone reader keeps to the
/// connections it just used. The map's shard lock guards each stack; no lock is held across
/// an await.
#[derive(Debug, Default)]
pub struct Conns {
    idle: DashMap<SocketAddr, Vec<TcpStream>>,
}

impl Conns {
    fn take(&self, addr: SocketAddr) -> Option<TcpStream> {
        self.idle.get_mut(&addr)?.pop()
    }

    /// Keeps `stream` unless `IDLE_PER_ADDR` are already idle; then it is closed, after the
    /// shard lock is released.
    fn put(&self, addr: SocketAddr, stream: TcpStream) {
        let mut idle = self.idle.entry(addr).or_default();
        if idle.len() < IDLE_PER_ADDR {
            idle.push(stream);
        }
    }
}

fn addrs(a: &AddrsInfo) -> Vec<SocketAddr> {
    [&a.addr1, &a.addr2]
        .into_iter()
        .filter(|ip| ip.port != 0)
        .map(|ip| SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(ip.addrs.0), ip.port)))
        .collect()
}

/// Fetches pages `[page_start, page_start + pages.len())` of `block_id` into `pages`
/// (each exactly 4096 bytes), verified, reusing an idle connection from `conns` when there
/// is one. Each attempt is bounded by the block timeout
/// schedule and the whole fetch by the block overall deadline; transient failures
/// (timeouts, connection and socket errors) are retried, alternating addresses.
pub async fn fetch_pages(
    conns: &Conns,
    service: &BlockService,
    block_id: BlockId,
    page_start: u32,
    pages: &mut [&mut [u8]],
    t: &ReqTimeouts,
) -> Result<(), ClientError> {
    let addrs = addrs(&service.addrs);
    if addrs.is_empty() {
        return Err(ClientError::NoAddress(Service::Blocks));
    }
    let count = u32::try_from(pages.len())
        .ok()
        .and_then(|n| n.checked_mul(TERN_PAGE_SIZE))
        .ok_or(ClientError::Malformed {
            what: "block fetch too large",
        })?;
    let offset = page_start
        .checked_mul(TERN_PAGE_SIZE)
        .ok_or(ClientError::Malformed {
            what: "block offset too large",
        })?;
    let req = encode_blocks_req(
        service.id,
        &FetchBlockWithCrcReq {
            file_id_unused: InodeId(0),
            block_id,
            block_crc_unused: Crc(0),
            offset,
            count,
        },
    )?;
    let start = Instant::now();
    let deadline =
        tokio::time::Instant::now()
            .checked_add(t.overall)
            .ok_or(ClientError::Malformed {
                what: "deadline overflow",
            })?;
    let mut last: Option<ClientError> = None;
    let mut schedule = timeouts::schedule(t);
    for addr in addrs.iter().copied().cycle() {
        let per_attempt = schedule.next().unwrap_or(t.max);
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let r = tokio::time::timeout(
            per_attempt.min(left),
            attempt_fetch(conns, addr, &req, block_id, pages),
        )
        .await;
        let e = match r {
            Ok(Ok(())) => {
                metrics::block_fetch(metrics::OK, start.elapsed(), u64::from(count));
                return Ok(());
            }
            Ok(Err(e)) if !retriable(&e) => {
                metrics::block_fetch(metrics::ERROR, start.elapsed(), 0);
                return Err(e);
            }
            Ok(Err(e)) => {
                // Upstream waits the next timeout before retrying a refused connection.
                let wait = per_attempt
                    .min(deadline.saturating_duration_since(tokio::time::Instant::now()));
                tracing::debug!(error = %e, %addr, block = block_id.0, "block fetch failed; retrying");
                tokio::time::sleep(wait).await;
                e
            }
            Err(_) => ClientError::Timeout {
                service: Service::Blocks,
                elapsed: start.elapsed(),
            },
        };
        last = Some(e);
    }
    metrics::block_fetch(metrics::TIMEOUT, start.elapsed(), 0);
    Err(match last {
        Some(ClientError::Timeout { .. }) | None => ClientError::Timeout {
            service: Service::Blocks,
            elapsed: start.elapsed(),
        },
        Some(e) => e,
    })
}

fn retriable(e: &ClientError) -> bool {
    matches!(e, ClientError::Connect { .. } | ClientError::Io(_))
}

async fn attempt_fetch(
    conns: &Conns,
    addr: SocketAddr,
    req: &[u8],
    block_id: BlockId,
    pages: &mut [&mut [u8]],
) -> Result<(), ClientError> {
    if let Some(stream) = conns.take(addr) {
        // The service closes idle connections (`-connection-idle-timeout`): a reused one
        // that fails at the transport level is replaced at once, without the retry wait.
        match exchange(stream, req, block_id, pages).await {
            Ok(stream) => {
                conns.put(addr, stream);
                return Ok(());
            }
            Err(e) if !retriable(&e) => return Err(e),
            Err(e) => {
                tracing::debug!(error = %e, %addr, "pooled block connection failed; reconnecting")
            }
        }
    }
    let t0 = Instant::now();
    let stream = TcpStream::connect(addr)
        .await
        .map_err(|source| ClientError::Connect {
            addr: addr.to_string(),
            source,
        })?;
    metrics::block_connect(t0.elapsed());
    stream.set_nodelay(true)?;
    let stream = exchange(stream, req, block_id, pages).await?;
    conns.put(addr, stream);
    Ok(())
}

/// Sends `req` on `stream` and reads its complete response into `pages`; hands the stream
/// back only then, so it is never reused mid-response.
async fn exchange(
    mut stream: TcpStream,
    req: &[u8],
    block_id: BlockId,
    pages: &mut [&mut [u8]],
) -> Result<TcpStream, ClientError> {
    let t0 = Instant::now();
    stream.write_all(req).await?;
    let mut header = [0u8; BLOCKS_RESP_HEADER_LEN];
    read_exact(&mut stream, &mut header).await?;
    metrics::block_header(t0.elapsed());
    match blocks_resp_kind(header).map_err(|_| ClientError::Malformed {
        what: "block response header",
    })? {
        None => {
            let mut code = [0u8; 2];
            read_exact(&mut stream, &mut code).await?;
            return Err(ClientError::Tern {
                code: ErrorCode(u16::from_le_bytes(code)),
                kind: "FETCH_BLOCK_WITH_CRC",
            });
        }
        Some(BlocksMessageKind::FetchBlockWithCrc) => {} // empty response body
        Some(_) => {
            return Err(ClientError::Malformed {
                what: "block response kind",
            });
        }
    }
    let mut crcs = vec![[0u8; CRC]; pages.len()];
    let mut sc = Scatter::new(pages.len());
    while !sc.done() {
        stream.readable().await?;
        let mut slices = sc.next_slices(pages, &mut crcs);
        match stream.try_read_vectored(&mut slices) {
            // Upstream retries EOF: a dropped connection is an I/O error.
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
            Ok(n) => {
                drop(slices);
                sc.advance(n);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
    }
    verify(block_id, pages, &crcs)?;
    Ok(stream)
}

async fn read_exact(stream: &mut TcpStream, buf: &mut [u8]) -> Result<(), ClientError> {
    stream.read_exact(buf).await?;
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn stream_of(data: &[u8]) -> Vec<u8> {
        data.chunks(PAGE)
            .flat_map(|p| {
                let mut v = p.to_vec();
                v.extend(rustern_core::crc::crc32c(p).to_le_bytes());
                v
            })
            .collect()
    }

    fn fill(stream: &[u8], cuts: &[usize], n: usize) -> (Vec<u8>, Vec<[u8; 4]>, Scatter) {
        let mut store = vec![0u8; n * PAGE];
        let mut crcs = vec![[0u8; 4]; n];
        let mut sc = Scatter::new(n);
        {
            let mut pages: Vec<&mut [u8]> = store.chunks_mut(PAGE).collect();
            let mut at = 0;
            for &c in cuts.iter().chain([&stream.len()]) {
                let c = c.clamp(at, stream.len());
                let mut chunk = &stream[at..c];
                while !chunk.is_empty() && !sc.done() {
                    let mut sl = sc.next_slices(&mut pages, &mut crcs);
                    assert!(sl.len() <= MAX_SLICES);
                    let k = io::Read::read_vectored(&mut chunk, &mut sl).unwrap();
                    drop(sl);
                    sc.advance(k);
                }
                at = c;
            }
        }
        (store, crcs, sc)
    }

    proptest! {
        #[test]
        fn any_split_strips_crcs(n in 1usize..80, seed: u64, mut cuts in proptest::collection::vec(0usize..400_000, 0..20)) {
            let mut rng = fastrand::Rng::with_seed(seed);
            let data: Vec<u8> = (0..n * PAGE).map(|_| rng.u8(..)).collect();
            let stream = stream_of(&data);
            cuts.sort_unstable();
            let (store, crcs, sc) = fill(&stream, &cuts, n);
            prop_assert!(sc.done());
            prop_assert!(store == data);
            for (i, c) in crcs.iter().enumerate() {
                prop_assert_eq!(u32::from_le_bytes(*c), rustern_core::crc::crc32c(&data[i * PAGE..(i + 1) * PAGE]));
            }
            let mut store = store;
            let pages: Vec<&mut [u8]> = store.chunks_mut(PAGE).collect();
            prop_assert!(verify(BlockId(1), &pages, &crcs).is_ok());
        }
    }

    #[test]
    fn mismatch_names_the_page() {
        let data = vec![7u8; 3 * PAGE];
        let mut stream = stream_of(&data);
        stream[2 * PAGE_WITH_CRC + 10] ^= 1;
        let (mut store, crcs, sc) = fill(&stream, &[], 3);
        assert!(sc.done());
        let pages: Vec<&mut [u8]> = store.chunks_mut(PAGE).collect();
        match verify(BlockId(9), &pages, &crcs) {
            Err(ClientError::CrcMismatch { block_id, page }) => {
                assert_eq!((block_id, page), (BlockId(9), 2))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn short_stream_is_malformed() {
        let mut data = vec![4u8];
        data.extend(stream_of(&vec![1u8; 4 * PAGE]));
        page_stream(&data).unwrap();
        data.truncate(data.len() - 1);
        assert!(matches!(
            page_stream(&data),
            Err(ClientError::Malformed { .. })
        ));
        assert!(page_stream(&[]).is_err());
    }

    #[test]
    fn page_count_comes_from_the_request() {
        // A server sending more than asked: the extra bytes are never read.
        let stream = stream_of(&vec![3u8; 3 * PAGE]);
        let (_, _, sc) = fill(&stream, &[], 2);
        assert!(sc.done());
        let mut sc = Scatter::new(2);
        sc.advance(usize::MAX);
        assert!(sc.done());
    }
}
