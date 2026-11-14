//! T078: `fetch_pages` against a fake TCP block service.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use common::rt;
use rustern_client::blocks::{Conns, fetch_pages};
use rustern_client::{ClientError, ReqTimeouts, Service};
use rustern_core::crc::crc32c;
use rustern_msgs::frame::{Reply, decode_blocks_req, encode_blocks_reply, encode_blocks_resp};
use rustern_msgs::wire::decode_exact;
use rustern_msgs::{
    AddrsInfo, BlockId, BlockService, BlockServiceId, BlocksMessageKind, ErrorCode,
    FetchBlockWithCrcReq, FetchBlockWithCrcResp, Ip, IpPort,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Request frame: protocol (4) + service id (8) + kind (1) + body (8+8+4+4+4).
const REQ_LEN: usize = 41;
const PAGE: usize = 4096;

#[derive(Clone, Copy)]
enum Mode {
    /// Serve every request on a connection until the client closes it.
    Serve,
    /// Serve one request, then close the connection.
    Once,
    /// As `Serve`, each response after a delay.
    Slow(Duration),
    Error(u16),
    BadCrc(usize),
    Stall,
    /// Close after half the pages for the first `n` connections, then serve.
    DropFirst(usize),
}

struct Fake {
    addr: SocketAddr,
    conns: Arc<AtomicUsize>,
    seen: Arc<std::sync::Mutex<Vec<(BlockServiceId, FetchBlockWithCrcReq)>>>,
}

fn block() -> Vec<u8> {
    (0..4 * PAGE).map(|i| (i % 251) as u8).collect()
}

async fn fake(mode: Mode) -> Fake {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let conns = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (c, s) = (Arc::clone(&conns), Arc::clone(&seen));
    tokio::spawn(async move {
        loop {
            let (stream, _) = l.accept().await.unwrap();
            let n = c.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(serve(stream, mode, n, Arc::clone(&s)));
        }
    });
    Fake { addr, conns, seen }
}

async fn serve(
    mut stream: TcpStream,
    mode: Mode,
    conn: usize,
    seen: Arc<std::sync::Mutex<Vec<(BlockServiceId, FetchBlockWithCrcReq)>>>,
) {
    loop {
        if !serve_one(&mut stream, mode, conn, &seen).await {
            return;
        }
    }
}

/// Serves one request; true when the connection stays open for the next.
async fn serve_one(
    stream: &mut TcpStream,
    mode: Mode,
    conn: usize,
    seen: &std::sync::Mutex<Vec<(BlockServiceId, FetchBlockWithCrcReq)>>,
) -> bool {
    let mut frame = [0u8; REQ_LEN];
    if stream.read_exact(&mut frame).await.is_err() {
        return false;
    }
    let req = decode_blocks_req(Bytes::copy_from_slice(&frame)).unwrap();
    assert_eq!(req.kind, BlocksMessageKind::FetchBlockWithCrc);
    let body: FetchBlockWithCrcReq = decode_exact(req.body).unwrap();
    seen.lock()
        .unwrap()
        .push((req.block_service_id, body.clone()));
    let mut out = match mode {
        Mode::Stall => {
            tokio::time::sleep(Duration::from_secs(30)).await;
            return false;
        }
        Mode::Error(code) => {
            let r = encode_blocks_reply(&Reply::Err(ErrorCode(code))).unwrap();
            let _ = stream.write_all(&r).await;
            return false;
        }
        Mode::Slow(d) => {
            tokio::time::sleep(d).await;
            encode_blocks_resp(&FetchBlockWithCrcResp {}).unwrap()
        }
        _ => encode_blocks_resp(&FetchBlockWithCrcResp {}).unwrap(),
    };
    let data = block();
    let (off, count) = (body.offset as usize, body.count as usize);
    for (i, page) in data[off..off + count].chunks(PAGE).enumerate() {
        out.extend_from_slice(page);
        let mut crc = crc32c(page);
        if matches!(mode, Mode::BadCrc(p) if p == i) {
            crc ^= 1;
        }
        out.extend_from_slice(&crc.to_le_bytes());
    }
    if matches!(mode, Mode::DropFirst(n) if conn < n) {
        out.truncate(out.len() / 2);
    }
    stream.write_all(&out).await.is_ok() && matches!(mode, Mode::Serve | Mode::Slow(_))
}

fn service(addrs: &[SocketAddr]) -> BlockService {
    let ip = |a: Option<&SocketAddr>| match a {
        Some(SocketAddr::V4(v4)) => IpPort {
            addrs: Ip(v4.ip().octets()),
            port: v4.port(),
        },
        _ => IpPort::default(),
    };
    BlockService {
        addrs: AddrsInfo {
            addr1: ip(addrs.first()),
            addr2: ip(addrs.get(1)),
        },
        id: BlockServiceId(77),
        ..BlockService::default()
    }
}

fn timeouts() -> ReqTimeouts {
    ReqTimeouts {
        initial: Duration::from_millis(50),
        max: Duration::from_millis(100),
        overall: Duration::from_millis(400),
    }
}

/// One fetch on a fresh connection pool.
async fn fetch(svc: &BlockService, page_start: u32, n: usize) -> Result<Vec<u8>, ClientError> {
    fetch_with(&Conns::default(), svc, page_start, n).await
}

async fn fetch_with(
    conns: &Conns,
    svc: &BlockService,
    page_start: u32,
    n: usize,
) -> Result<Vec<u8>, ClientError> {
    let mut bufs = vec![vec![0u8; PAGE]; n];
    let mut pages: Vec<&mut [u8]> = bufs.iter_mut().map(Vec::as_mut_slice).collect();
    fetch_pages(conns, svc, BlockId(5), page_start, &mut pages, &timeouts()).await?;
    Ok(bufs.concat())
}

#[test]
fn serves_requested_pages() {
    rt().block_on(async {
        let f = fake(Mode::Serve).await;
        let got = fetch(&service(&[f.addr]), 1, 2).await.unwrap();
        assert_eq!(got, &block()[PAGE..3 * PAGE]);
        let seen = f.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        let (svc, req) = &seen[0];
        assert_eq!(*svc, BlockServiceId(77));
        assert_eq!(
            (req.block_id, req.offset, req.count),
            (BlockId(5), 4096, 8192)
        );
    });
}

#[test]
fn error_reply_is_tern_error_without_retry() {
    rt().block_on(async {
        let f = fake(Mode::Error(ErrorCode::BLOCK_NOT_FOUND.0)).await;
        let r = fetch(&service(&[f.addr]), 0, 1).await;
        assert!(
            matches!(r, Err(ClientError::Tern { code, .. }) if code == ErrorCode::BLOCK_NOT_FOUND),
            "{r:?}"
        );
        assert_eq!(f.conns.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn bad_page_crc_is_reported_without_retry() {
    rt().block_on(async {
        let f = fake(Mode::BadCrc(2)).await;
        let r = fetch(&service(&[f.addr]), 0, 4).await;
        assert!(
            matches!(
                r,
                Err(ClientError::CrcMismatch {
                    block_id: BlockId(5),
                    page: 2
                })
            ),
            "{r:?}"
        );
        assert_eq!(f.conns.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn stall_times_out_at_the_deadline() {
    rt().block_on(async {
        let f = fake(Mode::Stall).await;
        let start = Instant::now();
        let r = fetch(&service(&[f.addr]), 0, 1).await;
        let took = start.elapsed();
        assert!(
            matches!(
                r,
                Err(ClientError::Timeout {
                    service: Service::Blocks,
                    ..
                })
            ),
            "{r:?}"
        );
        assert!(
            took >= Duration::from_millis(350) && took < Duration::from_secs(2),
            "{took:?}"
        );
        assert!(f.conns.load(Ordering::SeqCst) >= 2, "attempts were retried");
    });
}

#[test]
fn refused_is_connect_error_after_retries() {
    rt().block_on(async {
        let addr = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap()
        };
        let start = Instant::now();
        let r = fetch(&service(&[addr]), 0, 1).await;
        assert!(matches!(r, Err(ClientError::Connect { .. })), "{r:?}");
        assert!(start.elapsed() < Duration::from_secs(2));
    });
}

#[test]
fn dropped_stream_is_retried_and_alternates_addresses() {
    rt().block_on(async {
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap()
        };
        let f = fake(Mode::DropFirst(1)).await;
        let got = fetch(&service(&[f.addr, dead]), 0, 4).await.unwrap();
        assert_eq!(got, block());
        // Attempt 1: f (dropped); 2: dead (refused); 3: f (served).
        assert_eq!(f.conns.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn no_address_is_reported() {
    rt().block_on(async {
        let r = fetch(&service(&[]), 0, 1).await;
        assert!(
            matches!(r, Err(ClientError::NoAddress(Service::Blocks))),
            "{r:?}"
        );
    });
}

#[test]
fn sequential_fetches_reuse_one_connection() {
    rt().block_on(async {
        let f = fake(Mode::Serve).await;
        let (svc, conns) = (service(&[f.addr]), Conns::default());
        for _ in 0..3 {
            assert_eq!(fetch_with(&conns, &svc, 0, 4).await.unwrap(), block());
        }
        assert_eq!(f.conns.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn concurrent_fetches_use_separate_connections() {
    rt().block_on(async {
        let f = fake(Mode::Slow(Duration::from_millis(30))).await;
        let (svc, conns) = (service(&[f.addr]), Conns::default());
        let (a, b) = tokio::join!(
            fetch_with(&conns, &svc, 0, 4),
            fetch_with(&conns, &svc, 0, 4)
        );
        assert_eq!((a.unwrap(), b.unwrap()), (block(), block()));
        // Both connections went back to the pool: the next fetch opens none.
        fetch_with(&conns, &svc, 0, 4).await.unwrap();
        assert_eq!(f.conns.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn closed_pooled_connection_is_replaced_without_waiting() {
    rt().block_on(async {
        let f = fake(Mode::Once).await;
        let (svc, conns) = (service(&[f.addr]), Conns::default());
        fetch_with(&conns, &svc, 0, 4).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await; // the server's close arrives
        let start = Instant::now();
        assert_eq!(fetch_with(&conns, &svc, 0, 4).await.unwrap(), block());
        // A retry after a failed attempt first waits the attempt timeout (50 ms).
        let took = start.elapsed();
        assert!(took < Duration::from_millis(40), "{took:?}");
        assert_eq!(f.conns.load(Ordering::SeqCst), 2);
    });
}
