//! Async read-only TernFS client. No FUSE types here (Principle III).

pub mod addresses;
pub mod blocks;
pub mod bufpool;
pub mod cache;
pub mod config;
pub mod error;
pub mod metrics;
mod namespace;
pub mod read;
pub mod registry;
mod shard;
pub mod timeouts;
pub mod udp;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::time::{Duration, Instant};

use rustern_msgs::LocalShardsReq;
use tokio::task::JoinHandle;

pub use crate::addresses::ShardTable;
pub use crate::config::{ClientConfig, ReqTimeouts};
pub use crate::error::{ClientError, Service};
pub use crate::read::{FileHandle, MAX_READ, ReadBuf};
use crate::registry::registry_request;
use crate::udp::Udp;

/// A connection to one TernFS cluster. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    config: ClientConfig,
    udp: Udp,
    shards: RwLock<Arc<ShardTable>>,
    refresh: Mutex<Option<JoinHandle<()>>>,
    in_flight: namespace::InFlight,
    reads: read::Reads,
    /// Idle block-service connections.
    conns: blocks::Conns,
    /// When a shard failure last triggered a table refresh.
    failure_refresh: Mutex<Option<Instant>>,
}

/// Shortest interval between refreshes triggered by shard failures.
const FAILURE_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(t) = self
            .refresh
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            t.abort();
        }
    }
}

impl Client {
    /// Binds the UDP socket, fetches the shard table (within the shard overall deadline)
    /// and starts refreshing it every `registry_refresh`.
    pub async fn connect(config: ClientConfig) -> Result<Self, ClientError> {
        let udp = Udp::bind(usize::from(config.mtu)).await?;
        let table = fetch_shards(&config).await?;
        let reads = read::Reads::new(&config)?;
        let inner = Arc::new(Inner {
            config,
            reads,
            udp,
            shards: RwLock::new(Arc::new(table)),
            refresh: Mutex::new(None),
            in_flight: namespace::InFlight::new(),
            conns: blocks::Conns::default(),
            failure_refresh: Mutex::new(None),
        });
        let task = tokio::spawn(refresh_loop(Arc::downgrade(&inner)));
        *inner.refresh.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);
        Ok(Self { inner })
    }

    /// The configuration in use.
    pub fn config(&self) -> &ClientConfig {
        &self.inner.config
    }

    /// Addresses of `shard` from the current table.
    pub fn shard_addrs(&self, shard: u8) -> [Option<SocketAddr>; 2] {
        self.inner
            .shards
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(shard)
    }

    /// After a shard did not answer: refreshes the shard table in the background, at most
    /// once per second, so the retry loop picks up a moved shard.
    pub(crate) fn refresh_soon(&self) {
        {
            let mut last = self
                .inner
                .failure_refresh
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if last.is_some_and(|t| t.elapsed() < FAILURE_REFRESH_INTERVAL) {
                return;
            }
            *last = Some(Instant::now());
        }
        let inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let Some(inner) = inner.upgrade() else { return };
            match fetch_shards(&inner.config).await {
                Ok(table) => {
                    *inner.shards.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(table);
                }
                Err(e) => tracing::warn!(error = %e, "shard table refresh after a failure failed"),
            }
        });
    }

    /// Requests waiting for a UDP reply (tests).
    #[doc(hidden)]
    pub fn pending_requests(&self) -> usize {
        self.inner.udp.pending_len()
    }
}

async fn fetch_shards(config: &ClientConfig) -> Result<ShardTable, ClientError> {
    let resp = registry_request(&config.registry, &LocalShardsReq {}, config.shard.overall).await?;
    Ok(ShardTable::from_resp(&resp)?.with_overrides(&config.shard_overrides))
}

/// Refreshes the shard table; after `registry_alert_after` of failures logs one alert
/// (FR-016), and again only after a success and new failures.
async fn refresh_loop(inner: Weak<Inner>) {
    let Some(period) = inner.upgrade().map(|i| i.config.registry_refresh) else {
        return;
    };
    let mut tick = tokio::time::interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await; // the first tick is immediate; the table was just fetched
    let mut failing_since: Option<Instant> = None;
    let mut alerted = false;
    loop {
        tick.tick().await;
        let Some(inner) = inner.upgrade() else { return };
        let config = &inner.config;
        match fetch_shards(config).await {
            Ok(table) => {
                *inner.shards.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(table);
                failing_since = None;
                alerted = false;
            }
            Err(e) => {
                let since = *failing_since.get_or_insert_with(Instant::now);
                tracing::warn!(error = %e, registry = %config.registry, "shard table refresh failed");
                if !alerted && since.elapsed() > config.registry_alert_after {
                    tracing::error!(
                        alert = true,
                        registry = %config.registry,
                        failing_for = ?since.elapsed(),
                        "registry unreachable; using a stale shard table"
                    );
                    alerted = true;
                }
            }
        }
    }
}
