//! One shared UDP socket per client. A receive task decodes each datagram's frame and
//! hands the reply to whoever waits for its request id; everything else is counted and
//! dropped. Waiting is cancel-safe: dropping a [`Pending`] removes its entry.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use dashmap::DashMap;
use rustern_msgs::frame::{Reply, decode_udp_resp};
use rustern_msgs::{MAX_UDP_MTU, ShardMessageKind};
use tokio::net::UdpSocket;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::error::ClientError;
use crate::metrics;

type Waiters = DashMap<u64, oneshot::Sender<Reply<ShardMessageKind>>>;

/// The shared socket and its demultiplexer.
#[derive(Debug)]
pub struct Udp {
    sock: Arc<UdpSocket>,
    waiters: Arc<Waiters>,
    next_id: AtomicU64,
    mtu: usize,
    recv: JoinHandle<()>,
}

impl Udp {
    /// Binds an ephemeral port on all interfaces and starts the receive task.
    pub async fn bind(mtu: usize) -> Result<Self, ClientError> {
        Self::bind_to(
            "0.0.0.0:0".parse().map_err(|_| ClientError::Malformed {
                what: "bind address",
            })?,
            mtu,
        )
        .await
    }

    /// Binds `addr` and starts the receive task.
    pub async fn bind_to(addr: SocketAddr, mtu: usize) -> Result<Self, ClientError> {
        let sock = Arc::new(UdpSocket::bind(addr).await?);
        let waiters = Arc::new(Waiters::new());
        let recv = tokio::spawn(receive(Arc::clone(&sock), Arc::clone(&waiters)));
        Ok(Self {
            sock,
            waiters,
            next_id: AtomicU64::new(1),
            mtu,
            recv,
        })
    }

    /// Local address of the socket.
    pub fn local_addr(&self) -> Result<SocketAddr, ClientError> {
        Ok(self.sock.local_addr()?)
    }

    /// Allocates a request id and registers a waiter for it.
    pub fn begin(&self) -> Pending<'_> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.waiters.insert(id, tx);
        Pending { udp: self, id, rx }
    }

    /// Sends one frame; frames over the MTU are rejected before sending.
    pub async fn send(&self, frame: &[u8], to: SocketAddr) -> Result<(), ClientError> {
        if frame.len() > self.mtu {
            return Err(ClientError::TooLarge {
                len: frame.len(),
                mtu: self.mtu,
            });
        }
        self.sock.send_to(frame, to).await?;
        Ok(())
    }

    /// Requests waiting for a reply.
    pub fn pending_len(&self) -> usize {
        self.waiters.len()
    }
}

impl Drop for Udp {
    fn drop(&mut self) {
        self.recv.abort();
    }
}

/// A request id with a registered waiter.
#[derive(Debug)]
pub struct Pending<'a> {
    udp: &'a Udp,
    id: u64,
    rx: oneshot::Receiver<Reply<ShardMessageKind>>,
}

impl Pending<'_> {
    /// The request id.
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// The next reply for this id. Cancel-safe; call again after a timeout. After a
    /// reply was taken it waits forever until [`Pending::rearm`].
    pub async fn reply(&mut self) -> Result<Reply<ShardMessageKind>, ClientError> {
        if self.rx.is_terminated() {
            return std::future::pending().await;
        }
        (&mut self.rx).await.map_err(|_| ClientError::Malformed {
            what: "UDP receive task stopped",
        })
    }

    /// Waits for another reply under the same id (after an error reply that is retried).
    pub fn rearm(&mut self) {
        let (tx, rx) = oneshot::channel();
        self.udp.waiters.insert(self.id, tx);
        self.rx = rx;
    }
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.udp.waiters.remove(&self.id);
    }
}

async fn receive(sock: Arc<UdpSocket>, waiters: Arc<Waiters>) {
    let mut buf = vec![0u8; MAX_UDP_MTU];
    loop {
        let n = match sock.recv_from(&mut buf).await {
            Ok((n, _)) => n,
            Err(e) => {
                // ICMP errors (e.g. port unreachable) surface here on Linux; keep going.
                tracing::debug!(error = %e, "UDP receive error");
                continue;
            }
        };
        let Some(datagram) = buf.get(..n) else {
            continue;
        };
        match decode_udp_resp::<ShardMessageKind>(Bytes::copy_from_slice(datagram)) {
            Err(e) => {
                tracing::debug!(error = %e, "malformed datagram dropped");
                metrics::udp_malformed();
            }
            Ok(resp) => match waiters.remove(&resp.request_id) {
                Some((_, tx)) => {
                    // The waiter may have just given up; that is a late response.
                    if tx.send(resp.reply).is_err() {
                        metrics::udp_unmatched();
                    }
                }
                None => metrics::udp_unmatched(),
            },
        }
    }
}
