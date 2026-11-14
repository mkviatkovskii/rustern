//! Client configuration and its defaults (data-model.md "Configuration").

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

/// Size of one pooled read buffer: the largest read ([`crate::MAX_READ`]) plus room for
/// page alignment (R12).
pub const READ_BUF_SIZE: usize = crate::read::MAX_READ as usize + 8 * 1024;

/// Longest caller-facing deadline before a warning (DEV-006).
pub const MAX_DEADLINE: Duration = Duration::from_secs(60);

const MIB: usize = 1 << 20;

/// Per-attempt and overall timeouts of one service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReqTimeouts {
    /// First attempt's timeout.
    pub initial: Duration,
    /// Largest attempt timeout.
    pub max: Duration,
    /// Deadline for the whole request.
    pub overall: Duration,
}

impl ReqTimeouts {
    const fn new(initial_ms: u64, max_ms: u64, overall_ms: u64) -> Self {
        Self {
            initial: Duration::from_millis(initial_ms),
            max: Duration::from_millis(max_ms),
            overall: Duration::from_millis(overall_ms),
        }
    }
}

/// Settings of a [`crate::Client`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    /// Registry `host:port`.
    pub registry: String,
    /// Shard timeouts.
    pub shard: ReqTimeouts,
    /// CDC timeouts (accepted for ternfuse compatibility; the PoC sends no CDC requests).
    pub cdc: ReqTimeouts,
    /// Block service timeouts.
    pub block: ReqTimeouts,
    /// Largest UDP datagram.
    pub mtu: u16,
    /// Entries per readdir batch.
    pub readdir_batch: u32,
    /// Bytes of read buffers in total.
    pub memory_limit: usize,
    /// Bytes of read buffers per file.
    pub file_memory_limit: usize,
    /// How often the shard table is refreshed.
    pub registry_refresh: Duration,
    /// How long refreshes may fail before an alert is logged (FR-016).
    pub registry_alert_after: Duration,
    /// Attribute TTL for files.
    pub attr_ttl: Duration,
    /// Directory entry TTL.
    pub entry_ttl: Duration,
    /// Addresses used for these shards instead of the registry's (tests and debugging).
    pub shard_overrides: HashMap<u8, [SocketAddr; 2]>,
}

/// An invalid setting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid configuration: {0}")]
pub struct ConfigError(pub String);

impl ClientConfig {
    /// Defaults for `registry`.
    pub fn new(registry: impl Into<String>) -> Self {
        Self {
            registry: registry.into(),
            shard: ReqTimeouts::new(100, 2_000, 10_000),
            cdc: ReqTimeouts::new(1_000, 10_000, 60_000),
            block: ReqTimeouts::new(1_000, 10_000, 60_000), // DEV-006: upstream 5 min
            mtu: 1472,
            readdir_batch: 1000,
            memory_limit: 1 << 30,
            file_memory_limit: 32 * MIB,
            registry_refresh: Duration::from_secs(60),
            registry_alert_after: Duration::from_secs(60),
            attr_ttl: Duration::from_secs(3600),
            entry_ttl: Duration::from_millis(250),
            shard_overrides: HashMap::new(),
        }
    }

    /// Checks the settings; returns warnings for accepted but unusual values.
    pub fn validate(&self) -> Result<Vec<String>, ConfigError> {
        let mut warnings = Vec::new();
        for (name, t) in [
            ("shard", self.shard),
            ("cdc", self.cdc),
            ("block", self.block),
        ] {
            if t.initial.is_zero() || t.max.is_zero() || t.overall.is_zero() {
                return Err(ConfigError(format!("{name} timeouts must be positive")));
            }
            if t.initial > t.max {
                return Err(ConfigError(format!(
                    "initial {name} timeout {:?} exceeds max {:?}",
                    t.initial, t.max
                )));
            }
            if t.overall > MAX_DEADLINE {
                warnings.push(format!(
                    "overall {name} timeout {:?} exceeds {MAX_DEADLINE:?}; callers may wait that long",
                    t.overall
                ));
            }
        }
        for (name, d) in [
            ("registry refresh", self.registry_refresh),
            ("registry alert", self.registry_alert_after),
        ] {
            if d.is_zero() {
                return Err(ConfigError(format!("{name} interval must be positive")));
            }
        }
        if self.memory_limit < READ_BUF_SIZE {
            return Err(ConfigError(format!(
                "memory limit {} is below one read buffer ({READ_BUF_SIZE})",
                self.memory_limit
            )));
        }
        if self.file_memory_limit < 2 * MIB || self.file_memory_limit > self.memory_limit {
            return Err(ConfigError(format!(
                "per-file memory limit {} must be between 2 MiB and the memory limit {}",
                self.file_memory_limit, self.memory_limit
            )));
        }
        if self.mtu < 512 {
            return Err(ConfigError(format!("MTU {} is too small", self.mtu)));
        }
        if self.readdir_batch == 0 {
            return Err(ConfigError("readdir batch must be positive".into()));
        }
        Ok(warnings)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)] // test code
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn defaults() {
        let c = ClientConfig::new("r:1");
        assert_eq!(
            c.shard,
            ReqTimeouts {
                initial: ms(100),
                max: ms(2000),
                overall: ms(10_000)
            }
        );
        assert_eq!(
            c.cdc,
            ReqTimeouts {
                initial: ms(1000),
                max: ms(10_000),
                overall: ms(60_000)
            }
        );
        assert_eq!((c.mtu, c.readdir_batch), (1472, 1000));
        assert_eq!((c.memory_limit, c.file_memory_limit), (1 << 30, 32 << 20));
        assert_eq!(
            (c.registry_refresh, c.registry_alert_after),
            (ms(60_000), ms(60_000))
        );
        assert_eq!((c.attr_ttl, c.entry_ttl), (ms(3_600_000), ms(250)));
        let max_read = if cfg!(feature = "read2m") {
            2 << 20
        } else {
            1 << 20
        };
        assert_eq!(READ_BUF_SIZE, max_read + 8 * 1024);
        assert_eq!(c.validate(), Ok(vec![]));
    }

    /// DEV-006: block overall deadline is 60 s, not upstream's 5 min.
    #[test]
    fn block_deadline_default() {
        let c = ClientConfig::new("r:1");
        assert_eq!(
            c.block,
            ReqTimeouts {
                initial: ms(1000),
                max: ms(10_000),
                overall: ms(60_000)
            }
        );
    }

    #[test]
    fn validation() {
        let ok = ClientConfig::new("r:1");
        let bad = |f: fn(&mut ClientConfig)| {
            let mut c = ok.clone();
            f(&mut c);
            c.validate()
        };
        assert!(bad(|c| c.shard.initial = Duration::ZERO).is_err());
        assert!(bad(|c| c.block.overall = Duration::ZERO).is_err());
        assert!(bad(|c| c.shard.initial = c.shard.max + ms(1)).is_err());
        assert!(bad(|c| c.file_memory_limit = 2 * MIB - 1).is_err());
        assert!(bad(|c| c.file_memory_limit = c.memory_limit + 1).is_err());
        assert!(bad(|c| c.memory_limit = READ_BUF_SIZE - 1).is_err());
        assert!(bad(|c| c.registry_refresh = Duration::ZERO).is_err());
        assert!(bad(|c| c.registry_alert_after = Duration::ZERO).is_err());
        assert!(bad(|c| c.mtu = 511).is_err());
        assert!(bad(|c| c.readdir_batch = 0).is_err());
        assert!(bad(|c| c.file_memory_limit = 2 * MIB).is_ok());
        let warned = bad(|c| c.shard.overall = ms(61_000)).unwrap();
        assert_eq!(warned.len(), 1);
        assert!(warned[0].contains("shard"), "{warned:?}");
    }
}
