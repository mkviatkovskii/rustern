//! Client errors. No variant carries message bodies, so none can hold secrets (FR-017).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use rustern_core::inode::InvalidInode;
use rustern_core::span::SpanError;
use rustern_msgs::frame::FrameError;
use rustern_msgs::wire::{DecodeError, EncodeError};
use rustern_msgs::{BlockId, ErrorCode};

/// The service a request went to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    /// A metadata shard.
    Shard(u8),
    /// The registry.
    Registry,
    /// A block service.
    Blocks,
}

impl fmt::Display for Service {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shard(s) => write!(f, "shard {s}"),
            Self::Registry => f.write_str("registry"),
            Self::Blocks => f.write_str("block service"),
        }
    }
}

/// Why a client request failed.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The service answered with an error code.
    #[error("{kind} failed: {code}")]
    Tern {
        /// Error code from the service.
        code: ErrorCode,
        /// Upstream name of the request kind, e.g. `LOOKUP`.
        kind: &'static str,
    },
    /// No answer within the overall deadline.
    #[error("timed out after {elapsed:?} waiting for {service}")]
    Timeout {
        /// Service that did not answer.
        service: Service,
        /// Time spent.
        elapsed: Duration,
    },
    /// A response body did not decode.
    #[error("malformed response: {0}")]
    Decode(#[from] DecodeError),
    /// A request could not be encoded (e.g. a name longer than 255 bytes).
    #[error("cannot encode request: {0}")]
    Encode(#[from] EncodeError),
    /// A block page failed its CRC32C check.
    #[error("CRC mismatch in block {:#x} page {page}", block_id.0)]
    CrcMismatch {
        /// Block that failed.
        block_id: BlockId,
        /// Index of the 4 KiB page within the fetched range.
        page: u32,
    },
    /// TCP connection failed.
    #[error("cannot connect to {addr}: {source}")]
    Connect {
        /// Address tried.
        addr: String,
        /// Cause.
        source: std::io::Error,
    },
    /// Socket error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A response was well-formed on the wire but makes no sense.
    #[error("malformed response: {what}")]
    Malformed {
        /// What was wrong.
        what: &'static str,
    },
    /// An inode id with invalid type bits.
    #[error(transparent)]
    InvalidInode(#[from] InvalidInode),
    /// The request frame is larger than the MTU.
    #[error("request of {len} bytes exceeds the MTU of {mtu}")]
    TooLarge {
        /// Frame size.
        len: usize,
        /// Configured MTU.
        mtu: usize,
    },
    /// Neither address of the service is known.
    #[error("no address known for {0}")]
    NoAddress(Service),
    /// A span in a response failed validation.
    #[error("invalid span: {0}")]
    Span(#[from] SpanError),
    /// A read larger than the 1 MiB the kernel is told it may send.
    #[error("read of {0} bytes exceeds 1 MiB")]
    ReadTooLarge(u32),
    /// A data block is on a service that may not be read (stale, no-read,
    /// decommissioned) and there is no other copy (DEV-004).
    #[error("block {:#x} is not readable", block_id.0)]
    Unreadable {
        /// The block.
        block_id: BlockId,
    },
    /// The span list of a file has a gap.
    #[error("no span at offset {0}")]
    NoSpan(u64),
    /// An error shared by every caller waiting on one coalesced request.
    #[error(transparent)]
    Shared(Arc<ClientError>),
}

impl ClientError {
    /// Unwraps a shared error, or keeps it shared while other waiters hold it.
    pub fn shared(e: Arc<Self>) -> Self {
        Arc::try_unwrap(e).unwrap_or_else(Self::Shared)
    }

    /// The underlying error, looking through [`ClientError::Shared`].
    pub fn root(&self) -> &Self {
        match self {
            Self::Shared(e) => e.root(),
            e => e,
        }
    }
}

impl From<FrameError> for ClientError {
    fn from(e: FrameError) -> Self {
        match e {
            FrameError::Decode(d) => Self::Decode(d),
            FrameError::Protocol { .. } => Self::Malformed {
                what: "wrong protocol",
            },
            FrameError::UnknownKind(_) => Self::Malformed {
                what: "unknown message kind",
            },
            FrameError::TooLong { .. } => Self::Malformed {
                what: "response too long",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<ClientError> {
        vec![
            ClientError::Tern {
                code: ErrorCode::NAME_NOT_FOUND,
                kind: "LOOKUP",
            },
            ClientError::Timeout {
                service: Service::Shard(7),
                elapsed: Duration::from_secs(10),
            },
            ClientError::Decode(DecodeError::Truncated { needed: 3 }),
            ClientError::Encode(EncodeError::Invalid("x")),
            ClientError::CrcMismatch {
                block_id: BlockId(0xb10c),
                page: 2,
            },
            ClientError::Connect {
                addr: "10.0.0.1:10001".into(),
                source: std::io::Error::from(std::io::ErrorKind::ConnectionRefused),
            },
            ClientError::Io(std::io::Error::other("boom")),
            ClientError::Malformed { what: "wrong kind" },
            ClientError::InvalidInode(InvalidInode(5)),
            ClientError::TooLarge {
                len: 2000,
                mtu: 1472,
            },
            ClientError::NoAddress(Service::Shard(3)),
            ClientError::Span(SpanError::Stripes(16)),
            ClientError::ReadTooLarge(2 << 20),
            ClientError::Unreadable {
                block_id: BlockId(1),
            },
            ClientError::NoSpan(4096),
        ]
    }

    #[test]
    fn display_names_what_is_involved() {
        let d: Vec<String> = all().iter().map(ToString::to_string).collect();
        assert_eq!(d[0], "LOOKUP failed: NAME_NOT_FOUND");
        assert_eq!(d[1], "timed out after 10s waiting for shard 7");
        assert!(d[4].contains("block 0xb10c page 2"), "{}", d[4]);
        assert!(
            d[5].starts_with("cannot connect to 10.0.0.1:10001"),
            "{}",
            d[5]
        );
        assert!(d[8].contains("0x5"), "{}", d[8]);
        assert!(d[10].contains("shard 3"), "{}", d[10]);
    }

    #[test]
    fn debug_has_no_payload_bytes() {
        // Variants hold codes, ids and addresses only; check nothing renders as a byte list.
        for e in all() {
            let d = format!("{e:?}");
            assert!(!d.contains("[0,") && !d.contains("Bytes"), "{d}");
        }
    }

    #[test]
    fn frame_errors_convert() {
        assert!(matches!(
            ClientError::from(FrameError::UnknownKind(9)),
            ClientError::Malformed { .. }
        ));
        assert!(matches!(
            ClientError::from(FrameError::Decode(DecodeError::TrailingBytes(1))),
            ClientError::Decode(_)
        ));
    }
}
