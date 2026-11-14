//! Shard address table from the registry (`LocalShardsReq`, 256 entries).

use std::collections::HashMap;
use std::net::{SocketAddr, SocketAddrV4};

use rustern_msgs::{IpPort, LocalShardsResp};

use crate::error::ClientError;

/// Number of shards.
pub const SHARDS: usize = 256;

/// Two addresses per shard; `None` where the registry has no address (port 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardTable {
    shards: Vec<[Option<SocketAddr>; 2]>,
}

fn addr(ip: &IpPort) -> Option<SocketAddr> {
    (ip.port != 0).then(|| SocketAddr::V4(SocketAddrV4::new(ip.addrs.0.into(), ip.port)))
}

impl ShardTable {
    /// Builds the table; the response must list exactly 256 shards.
    pub fn from_resp(resp: &LocalShardsResp) -> Result<Self, ClientError> {
        if resp.shards.len() != SHARDS {
            return Err(ClientError::Malformed {
                what: "LocalShardsResp without 256 shards",
            });
        }
        let shards = resp
            .shards
            .iter()
            .map(|s| [addr(&s.addrs.addr1), addr(&s.addrs.addr2)])
            .collect();
        Ok(Self { shards })
    }

    /// Replaces the addresses of the shards in `overrides`.
    #[must_use]
    pub fn with_overrides(mut self, overrides: &HashMap<u8, [SocketAddr; 2]>) -> Self {
        for (shard, [a, b]) in overrides {
            if let Some(e) = self.shards.get_mut(usize::from(*shard)) {
                *e = [Some(*a), Some(*b)];
            }
        }
        self
    }

    /// Addresses of `shard`.
    pub fn get(&self, shard: u8) -> [Option<SocketAddr>; 2] {
        self.shards
            .get(usize::from(shard))
            .copied()
            .unwrap_or([None, None])
    }
}
