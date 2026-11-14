//! Read buffers (R12), pooled by `deadpool`: handed out the first time none is idle,
//! returned on drop, never freed. The pool's size is the global bound on live buffers
//! (FR-012): a read waits in [`BufPool::get`] while all of them are in use.
//!
//! The buffers are slices of one anonymous mapping of the whole limit, advised
//! `MADV_HUGEPAGE` (transparent huge pages are often `madvise`-only), so copying a 1 MiB
//! read touches a few TLB entries instead of 256. The mapping reserves address space only;
//! memory is committed as buffers are first written.
//!
//! Last in, first out: after a burst of readers has filled the pool, a lone reader keeps
//! reusing the few buffers still in the CPU cache instead of cycling through all of them
//! (a FIFO pool cost one reader 8% of its throughput).

use std::io;
use std::slice::ChunksExactMut;
use std::sync::{Mutex, PoisonError};

use deadpool::managed::{self, Metrics, Object, Pool, QueueMode, RecycleResult};
use memmap2::{Advice, MmapMut};

use crate::config::READ_BUF_SIZE;

/// Hands out the not yet used `READ_BUF_SIZE` slices of the mapping; a returned buffer is
/// reused as it is.
#[derive(Debug)]
pub struct Buffers {
    unused: Mutex<ChunksExactMut<'static, u8>>,
}

impl managed::Manager for Buffers {
    type Type = &'static mut [u8];
    type Error = io::Error;

    async fn create(&self) -> io::Result<&'static mut [u8]> {
        self.unused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next()
            .ok_or_else(|| io::Error::other("buffer mapping exhausted"))
    }

    async fn recycle(&self, _: &mut &'static mut [u8], _: &Metrics) -> RecycleResult<io::Error> {
        Ok(())
    }
}

/// A pool of read buffers.
pub type BufPool = Pool<Buffers>;

/// A buffer from a [`BufPool`]; goes back on drop. Contents are unspecified.
pub type PooledBuf = Object<Buffers>;

/// A pool of at most `capacity` buffers (`memory_limit / READ_BUF_SIZE`), none committed
/// yet. The mapping lives as long as the process.
pub fn new(capacity: usize) -> io::Result<BufPool> {
    let capacity = capacity.max(1);
    let len = capacity
        .checked_mul(READ_BUF_SIZE)
        .ok_or_else(|| io::Error::other("memory limit too large"))?;
    let map = MmapMut::map_anon(len)?;
    if let Err(e) = map.advise(Advice::HugePage) {
        tracing::debug!(error = %e, "MADV_HUGEPAGE refused; read buffers use small pages");
    }
    let map: &'static mut MmapMut = Box::leak(Box::new(map));
    let slab: &'static mut [u8] = map;
    // `as_chunks_mut` would hand out arrays; reads take slices.
    #[allow(clippy::chunks_exact_to_as_chunks)]
    Pool::builder(Buffers {
        unused: Mutex::new(slab.chunks_exact_mut(READ_BUF_SIZE)),
    })
    .max_size(capacity)
    .queue_mode(QueueMode::Lifo)
    .build()
    .map_err(io::Error::other)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // test code
mod tests {
    use std::time::Duration;

    use deadpool::managed::{PoolError, Timeouts};

    use super::*;

    /// A buffer, or `None` when all are in use (no waiting).
    async fn try_get(pool: &BufPool) -> Option<PooledBuf> {
        let now = Timeouts {
            wait: Some(Duration::ZERO),
            ..Timeouts::default()
        };
        match pool.timeout_get(&now).await {
            Ok(b) => Some(b),
            Err(PoolError::Timeout(_)) => None,
            Err(e) => panic!("{e}"),
        }
    }

    #[tokio::test]
    async fn lazy_reused_and_kept() {
        let pool = new(8).unwrap();
        assert_eq!(pool.status().size, 0, "nothing handed out yet");
        let b = pool.get().await.unwrap();
        assert_eq!(b.len(), READ_BUF_SIZE);
        drop(b);
        let b = pool.get().await.unwrap();
        assert_eq!(pool.status().size, 1, "reused");
        drop(b);

        let mut burst = Vec::new();
        for _ in 0..5 {
            burst.push(pool.get().await.unwrap());
        }
        drop(burst);
        assert_eq!(pool.status().available, 5, "kept after the burst");
        let mut again = Vec::new();
        for _ in 0..5 {
            again.push(pool.get().await.unwrap());
        }
        assert_eq!(pool.status().size, 5, "no new buffer");
    }

    #[tokio::test]
    async fn last_returned_comes_back_first() {
        let pool = new(4).unwrap();
        let (a, b) = (pool.get().await.unwrap(), pool.get().await.unwrap());
        let last = b.as_ptr();
        drop(a);
        drop(b);
        assert_eq!(pool.get().await.unwrap().as_ptr(), last);
    }

    #[tokio::test]
    async fn bounded_by_capacity() {
        let pool = new(2).unwrap();
        let mut held = (try_get(&pool).await.unwrap(), try_get(&pool).await.unwrap());
        held.0.fill(1);
        held.1.fill(2);
        assert!(held.0.iter().all(|&x| x == 1), "buffers do not overlap");
        assert!(try_get(&pool).await.is_none(), "a third waits");
        drop(held);
        assert!(try_get(&pool).await.is_some());
        assert_eq!(pool.status().size, 2);
    }
}
