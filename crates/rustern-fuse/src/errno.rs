//! The only place a [`ClientError`] becomes an errno (research R8).

use fuser::Errno;
use rustern_client::{ClientError, Service};
use rustern_msgs::ErrorCode;

/// Errno for a TernFS error code.
pub fn errno_of_code(code: ErrorCode) -> Errno {
    match code {
        ErrorCode::FILE_NOT_FOUND
        | ErrorCode::DIRECTORY_NOT_FOUND
        | ErrorCode::NAME_NOT_FOUND
        | ErrorCode::EDGE_NOT_FOUND => Errno::ENOENT,
        ErrorCode::TYPE_IS_DIRECTORY => Errno::EISDIR,
        ErrorCode::TYPE_IS_NOT_DIRECTORY => Errno::ENOTDIR,
        ErrorCode::TIMEOUT => Errno::ETIMEDOUT,
        ErrorCode::NOT_AUTHORISED | ErrorCode::BAD_COOKIE => Errno::EACCES,
        _ => Errno::EIO,
    }
}

/// Errno for a client error. No wildcard arm: a new variant must be mapped here.
pub fn errno_of(e: &ClientError) -> Errno {
    match e {
        ClientError::Tern { code, .. } => errno_of_code(*code),
        ClientError::Timeout { service, .. } => match service {
            Service::Shard(_) | Service::Registry => Errno::ETIMEDOUT,
            Service::Blocks => Errno::EIO,
        },
        ClientError::Decode(_)
        | ClientError::Encode(_)
        | ClientError::CrcMismatch { .. }
        | ClientError::Connect { .. }
        | ClientError::Io(_)
        | ClientError::Malformed { .. }
        | ClientError::InvalidInode(_)
        | ClientError::TooLarge { .. }
        | ClientError::NoAddress(_)
        | ClientError::Span(_)
        | ClientError::ReadTooLarge(_)
        | ClientError::Unreadable { .. }
        | ClientError::NoSpan(_) => Errno::EIO,
        ClientError::Shared(e) => errno_of(e),
    }
}

/// Label value for `rustern_fuse_ops_total{errno}`; one of [`ERRNO_NAMES`].
pub fn errno_name(e: Errno) -> &'static str {
    ERRNOS
        .iter()
        .find(|(x, _)| *x == e)
        .map_or("OTHER", |(_, n)| n)
}

const ERRNOS: [(Errno, &str); 9] = [
    (Errno::ENOENT, "ENOENT"),
    (Errno::EISDIR, "EISDIR"),
    (Errno::ENOTDIR, "ENOTDIR"),
    (Errno::ETIMEDOUT, "ETIMEDOUT"),
    (Errno::EACCES, "EACCES"),
    (Errno::EIO, "EIO"),
    (Errno::ENOSYS, "ENOSYS"),
    (Errno::EINVAL, "EINVAL"),
    (Errno::EROFS, "EROFS"),
];

/// Every `errno` label value.
pub const ERRNO_NAMES: [&str; 10] = [
    "ENOENT",
    "EISDIR",
    "ENOTDIR",
    "ETIMEDOUT",
    "EACCES",
    "EIO",
    "ENOSYS",
    "EINVAL",
    "EROFS",
    "OTHER",
];

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rustern_msgs::BlockId;
    use rustern_msgs::wire::DecodeError;

    use super::*;

    fn tern(code: ErrorCode) -> ClientError {
        ClientError::Tern {
            code,
            kind: "LOOKUP",
        }
    }

    #[test]
    fn codes() {
        for c in [
            ErrorCode::FILE_NOT_FOUND,
            ErrorCode::DIRECTORY_NOT_FOUND,
            ErrorCode::NAME_NOT_FOUND,
            ErrorCode::EDGE_NOT_FOUND,
        ] {
            assert_eq!(errno_of(&tern(c)), Errno::ENOENT, "{c:?}");
        }
        assert_eq!(errno_of(&tern(ErrorCode::TYPE_IS_DIRECTORY)), Errno::EISDIR);
        assert_eq!(
            errno_of(&tern(ErrorCode::TYPE_IS_NOT_DIRECTORY)),
            Errno::ENOTDIR
        );
        assert_eq!(errno_of(&tern(ErrorCode::TIMEOUT)), Errno::ETIMEDOUT);
        assert_eq!(errno_of(&tern(ErrorCode::NOT_AUTHORISED)), Errno::EACCES);
        assert_eq!(errno_of(&tern(ErrorCode::BAD_COOKIE)), Errno::EACCES);
        for c in [
            ErrorCode::INTERNAL_ERROR,
            ErrorCode::MALFORMED_RESPONSE,
            ErrorCode(9999),
        ] {
            assert_eq!(errno_of(&tern(c)), Errno::EIO, "{c:?}");
        }
    }

    #[test]
    fn other_errors() {
        let timeout = |service| ClientError::Timeout {
            service,
            elapsed: Duration::from_secs(1),
        };
        assert_eq!(errno_of(&timeout(Service::Shard(3))), Errno::ETIMEDOUT);
        assert_eq!(errno_of(&timeout(Service::Registry)), Errno::ETIMEDOUT);
        assert_eq!(errno_of(&timeout(Service::Blocks)), Errno::EIO);
        let eio = [
            ClientError::Decode(DecodeError::Truncated { needed: 1 }),
            ClientError::CrcMismatch {
                block_id: BlockId(1),
                page: 0,
            },
            ClientError::Connect {
                addr: "a:1".into(),
                source: std::io::ErrorKind::Other.into(),
            },
            ClientError::Io(std::io::ErrorKind::Other.into()),
            ClientError::Malformed { what: "x" },
            ClientError::InvalidInode(rustern_core::inode::InvalidInode(0)),
            ClientError::TooLarge { len: 2, mtu: 1 },
            ClientError::NoAddress(Service::Shard(0)),
            ClientError::Span(rustern_core::span::SpanError::NoDataBlocks),
            ClientError::ReadTooLarge(2 << 20),
            ClientError::Unreadable {
                block_id: BlockId(1),
            },
            ClientError::NoSpan(0),
        ];
        for e in &eio {
            assert_eq!(errno_of(e), Errno::EIO, "{e:?}");
        }
    }

    #[test]
    fn names() {
        assert_eq!(errno_name(Errno::ENOENT), "ENOENT");
        assert_eq!(errno_name(Errno::EPERM), "OTHER");
        for (_, n) in ERRNOS {
            assert!(ERRNO_NAMES.contains(&n));
        }
    }
}
