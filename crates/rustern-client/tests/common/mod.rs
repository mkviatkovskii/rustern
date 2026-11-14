//! In-process fakes shared by the client integration tests.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use rustern_client::{Client, ClientConfig, ReqTimeouts};
use rustern_msgs::frame::{
    Reply, UdpReq, UdpResp, decode_udp_req, encode_registry_reply, encode_registry_resp,
    encode_shard_resp, encode_udp_resp,
};
use rustern_msgs::{
    AddrsInfo, ErrorCode, InodeId, Ip, IpPort, LocalShardsResp, LookupReq, LookupResp,
    REGISTRY_RESP_PROTOCOL_VERSION, RegistryMessageKind, ShardInfo, ShardMessageKind,
    ShardResponse, TernTime,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::task::JoinHandle;

/// A shard that answers only when the test tells it to.
pub struct FakeShard {
    pub sock: UdpSocket,
}

impl FakeShard {
    pub async fn bind() -> Self {
        Self {
            sock: UdpSocket::bind("127.0.0.1:0").await.unwrap(),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.sock.local_addr().unwrap()
    }

    pub async fn recv(&self) -> (UdpReq<ShardMessageKind>, SocketAddr) {
        let mut buf = vec![0u8; 9000];
        let (n, from) = self.sock.recv_from(&mut buf).await.unwrap();
        buf.truncate(n);
        (decode_udp_req(Bytes::from(buf)).unwrap(), from)
    }

    /// The next request, or `None` after `wait`.
    pub async fn try_recv(&self, wait: Duration) -> Option<(UdpReq<ShardMessageKind>, SocketAddr)> {
        tokio::time::timeout(wait, self.recv()).await.ok()
    }

    pub async fn reply<R: ShardResponse>(&self, id: u64, resp: &R, to: SocketAddr) {
        self.send_raw(&encode_shard_resp(id, resp).unwrap(), to)
            .await;
    }

    pub async fn reply_err(&self, id: u64, code: ErrorCode, to: SocketAddr) {
        let frame = encode_udp_resp(&UdpResp::<ShardMessageKind> {
            request_id: id,
            reply: Reply::Err(code),
        })
        .unwrap();
        self.send_raw(&frame, to).await;
    }

    pub async fn send_raw(&self, bytes: &[u8], to: SocketAddr) {
        self.sock.send_to(bytes, to).await.unwrap();
    }
}

pub fn ip_port(a: Option<SocketAddr>) -> IpPort {
    match a {
        Some(SocketAddr::V4(a)) => IpPort {
            addrs: Ip(a.ip().octets()),
            port: a.port(),
        },
        Some(SocketAddr::V6(_)) => panic!("IPv4 only"),
        None => IpPort::default(),
    }
}

/// 256 shards, all at `a1` / `a2`.
pub fn shards_resp(a1: Option<SocketAddr>, a2: Option<SocketAddr>) -> LocalShardsResp {
    shards_resp_with(|_| (a1, a2))
}

pub fn shards_resp_with(
    f: impl Fn(u8) -> (Option<SocketAddr>, Option<SocketAddr>),
) -> LocalShardsResp {
    LocalShardsResp {
        shards: (0..=255u8)
            .map(|i| {
                let (a1, a2) = f(i);
                ShardInfo {
                    addrs: AddrsInfo {
                        addr1: ip_port(a1),
                        addr2: ip_port(a2),
                    },
                    last_seen: TernTime(0),
                }
            })
            .collect(),
    }
}

/// How the fake registry behaves.
#[derive(Clone)]
pub enum Registry {
    /// Answers every request with this response.
    Answer(LocalShardsResp),
    /// Answers with the current table and counts requests.
    Shared(Arc<Mutex<LocalShardsResp>>, Arc<AtomicUsize>),
    /// Accepts and reads, never answers.
    Silent,
    /// Sends a header announcing `len` bytes, then nothing.
    Length(u32),
    /// Answers with this reply (an error, or a response of another kind).
    Reply(Reply<RegistryMessageKind>),
}

/// Starts a fake registry; aborting the handle closes its port.
pub async fn fake_registry(mode: Registry) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let task = tokio::spawn(async move {
        loop {
            let (conn, _) = listener.accept().await.unwrap();
            tokio::spawn(serve_registry(conn, mode.clone()));
        }
    });
    (addr, task)
}

async fn serve_registry(mut conn: TcpStream, mode: Registry) {
    let mut header = [0u8; 8];
    if conn.read_exact(&mut header).await.is_err() {
        return;
    }
    let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let mut body = BytesMut::zeroed(len);
    if conn.read_exact(&mut body).await.is_err() {
        return;
    }
    match mode {
        Registry::Answer(resp) => {
            let _ = conn.write_all(&encode_registry_resp(&resp).unwrap()).await;
        }
        Registry::Shared(resp, count) => {
            count.fetch_add(1, Ordering::SeqCst);
            let resp = resp.lock().unwrap().clone();
            let _ = conn.write_all(&encode_registry_resp(&resp).unwrap()).await;
        }
        Registry::Silent => tokio::time::sleep(Duration::from_secs(3600)).await,
        Registry::Length(len) => {
            let mut h = REGISTRY_RESP_PROTOCOL_VERSION.to_le_bytes().to_vec();
            h.extend_from_slice(&len.to_le_bytes());
            let _ = conn.write_all(&h).await;
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
        Registry::Reply(reply) => {
            let _ = conn
                .write_all(&encode_registry_reply(&reply).unwrap())
                .await;
        }
    }
}

/// Defaults with short shard timeouts: 20 ms initial, 100 ms max, 1 s overall.
pub fn config(registry: String) -> ClientConfig {
    let mut c = ClientConfig::new(registry);
    c.shard = ReqTimeouts {
        initial: Duration::from_millis(20),
        max: Duration::from_millis(100),
        overall: Duration::from_secs(1),
    };
    c
}

/// A client whose every shard is at `a1` / `a2`.
pub async fn client(a1: Option<SocketAddr>, a2: Option<SocketAddr>) -> (Client, JoinHandle<()>) {
    client_with(a1, a2, |_| {}).await
}

pub async fn client_with(
    a1: Option<SocketAddr>,
    a2: Option<SocketAddr>,
    tweak: impl FnOnce(&mut ClientConfig),
) -> (Client, JoinHandle<()>) {
    let (registry, task) = fake_registry(Registry::Answer(shards_resp(a1, a2))).await;
    let mut c = config(registry);
    tweak(&mut c);
    (Client::connect(c).await.unwrap(), task)
}

pub fn lookup(name: &str) -> LookupReq {
    LookupReq {
        dir_id: InodeId(0x2000_0000_0000_0000),
        name: Bytes::copy_from_slice(name.as_bytes()),
    }
}

pub fn lookup_resp(target: u64) -> LookupResp {
    LookupResp {
        target_id: InodeId(target),
        creation_time: TernTime(7),
    }
}

pub fn localhost(a: SocketAddr) -> bool {
    a.ip() == IpAddr::from([127, 0, 0, 1])
}

/// A current-thread runtime.
pub fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Runs `f` on a current-thread runtime with a local Prometheus recorder.
pub fn with_recorder<F: Future>(f: F) -> (F::Output, PrometheusHandle) {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    let out = metrics::with_local_recorder(&recorder, || rt().block_on(f));
    (out, handle)
}

/// Polls `cond` every 5 ms for up to 2 s.
pub async fn eventually(mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    cond()
}

/// Value of the first rendered sample line starting with `prefix`.
pub fn sample(rendered: &str, prefix: &str) -> Option<f64> {
    rendered
        .lines()
        .find(|l| l.starts_with(prefix))
        .and_then(|l| l.rsplit(' ').next())
        .and_then(|v| v.parse().ok())
}
