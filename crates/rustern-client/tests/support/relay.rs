//! A test-only UDP relay between the client and one shard: drops, duplicates, delays and
//! damages datagrams (T100).
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// Fault rates (0..1) and the largest added delay.
#[derive(Debug, Clone, Copy)]
pub struct Faults {
    /// Datagrams dropped, each direction.
    pub loss: f64,
    /// Datagrams sent twice, each direction.
    pub dup: f64,
    /// Each copy is delayed by up to this, so datagrams are reordered.
    pub max_delay: Duration,
    /// Responses damaged (see [`Damage`]).
    pub damage: f64,
}

/// How a response is damaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Damage {
    /// Cut to a random shorter length.
    Truncate,
    /// Request id changed.
    WrongId,
    /// Protocol number changed.
    WrongProtocol,
    /// A `ReadDirResp` list length set beyond the datagram.
    LongLength,
}

const DAMAGES: [Damage; 4] = [
    Damage::Truncate,
    Damage::WrongId,
    Damage::WrongProtocol,
    Damage::LongLength,
];

/// Protocol (4) + request id (8) + kind (1).
const HEADER: usize = 13;
/// `ReadDirResp`: next hash (8) then the u16 list length.
const READDIR_LEN_AT: usize = HEADER + 8;

fn damage(d: Damage, mut b: Vec<u8>, rng: &mut fastrand::Rng) -> Vec<u8> {
    match d {
        Damage::Truncate => {
            let n = b.len();
            b.truncate(rng.usize(0..n.max(1)));
        }
        Damage::WrongId => {
            if b.len() >= HEADER {
                b[4] ^= 0x5a;
            }
        }
        Damage::WrongProtocol => {
            if !b.is_empty() {
                b[0] ^= 0xff;
            }
        }
        Damage::LongLength => {
            if b.len() >= READDIR_LEN_AT + 2 {
                b[READDIR_LEN_AT] = 0xff;
                b[READDIR_LEN_AT + 1] = 0xff;
            }
        }
    }
    b
}

/// A running relay; stops on drop.
pub struct Relay {
    /// Where the client sends.
    pub addr: SocketAddr,
    task: JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn send_later(
    sock: &Arc<UdpSocket>,
    bytes: Vec<u8>,
    to: SocketAddr,
    f: Faults,
    rng: &mut fastrand::Rng,
) {
    let copies = if rng.f64() < f.dup { 2 } else { 1 };
    for _ in 0..copies {
        if rng.f64() < f.loss {
            continue;
        }
        let delay = f.max_delay.mul_f64(rng.f64());
        let (sock, bytes) = (Arc::clone(sock), bytes.clone());
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = sock.send_to(&bytes, to).await;
        });
    }
}

/// Starts a relay to `target`.
pub async fn relay(target: SocketAddr, f: Faults, seed: u64) -> Relay {
    let front = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let back = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let addr = front.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut rng = fastrand::Rng::with_seed(seed);
        let mut client: Option<SocketAddr> = None;
        let (mut a, mut b) = (vec![0u8; 65536], vec![0u8; 65536]);
        loop {
            tokio::select! {
                r = front.recv_from(&mut a) => {
                    let Ok((n, from)) = r else { continue };
                    client = Some(from);
                    send_later(&back, a[..n].to_vec(), target, f, &mut rng);
                }
                r = back.recv_from(&mut b) => {
                    let (Ok((n, _)), Some(to)) = (r, client) else { continue };
                    let mut bytes = b[..n].to_vec();
                    if rng.f64() < f.damage {
                        bytes = damage(DAMAGES[rng.usize(0..DAMAGES.len())], bytes, &mut rng);
                    }
                    send_later(&front, bytes, to, f, &mut rng);
                }
            }
        }
    });
    Relay { addr, task }
}
