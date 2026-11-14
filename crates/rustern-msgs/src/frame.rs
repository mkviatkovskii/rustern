//! Request and response frames around message bodies, as upstream `go/client` writes and
//! reads them (`client.go`, `registryreq.go`, `blocksreq.go`):
//!
//! - shard/CDC (UDP): `u32 protocol, u64 request_id, u8 kind, body`; a response of kind
//!   `ERROR` carries exactly a `u16` error code.
//! - registry (TCP): request `u32 protocol, u32 len, u8 kind, body` with `len = 1 + body`;
//!   response `u32 protocol, u32 len, data` where `data` starts with the kind byte.
//! - blocks (TCP): request `u32 protocol, u64 block_service_id, u8 kind, body`; response
//!   `u32 protocol, u8 kind, body` (or `ERROR`, `u16`).
//!
//! Bodies stay as [`Bytes`]; callers decode them with the type for the kind.

use bytes::{BufMut, Bytes};

use crate::generated::{
    BLOCKS_REQ_PROTOCOL_VERSION, BLOCKS_RESP_PROTOCOL_VERSION, BlockServiceId, BlocksMessageKind,
    BlocksRequest, BlocksResponse, CDC_REQ_PROTOCOL_VERSION, CDC_RESP_PROTOCOL_VERSION,
    CdcMessageKind, ERROR_KIND, ErrorCode, REGISTRY_REQ_PROTOCOL_VERSION,
    REGISTRY_RESP_PROTOCOL_VERSION, RegistryMessageKind, RegistryRequest, RegistryResponse,
    SHARD_REQ_PROTOCOL_VERSION, SHARD_RESP_PROTOCOL_VERSION, ShardMessageKind, ShardRequest,
    ShardResponse,
};
use crate::wire::{Decode, DecodeError, Encode, EncodeError, decode_exact};

/// Why a frame was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The frame header or error body is truncated or has trailing bytes.
    #[error("malformed frame: {0}")]
    Decode(#[from] DecodeError),
    /// The protocol word does not match the service.
    #[error("protocol {got:#x}, expected {expected:#x}")]
    Protocol {
        /// Protocol of the service.
        expected: u32,
        /// Protocol in the frame.
        got: u32,
    },
    /// The kind byte is not a message kind of the service.
    #[error("unknown message kind {0}")]
    UnknownKind(u8),
    /// A registry response announces more bytes than allowed.
    #[error("registry response of {len} bytes exceeds {max}")]
    TooLong {
        /// Announced length.
        len: u32,
        /// Limit.
        max: usize,
    },
}

/// A message-kind enum of one service, with that service's protocol words.
pub trait Kind: Copy + Sized {
    /// Protocol word of requests.
    const REQ_PROTOCOL: u32;
    /// Protocol word of responses.
    const RESP_PROTOCOL: u32;
    /// The kind byte.
    fn to_u8(self) -> u8;
    /// The kind for a byte, if the service has one.
    fn from_u8(v: u8) -> Option<Self>;
}

macro_rules! kind {
    ($ty:ty, $req:expr, $resp:expr) => {
        impl Kind for $ty {
            const REQ_PROTOCOL: u32 = $req;
            const RESP_PROTOCOL: u32 = $resp;
            fn to_u8(self) -> u8 {
                self as u8
            }
            fn from_u8(v: u8) -> Option<Self> {
                Self::try_from(v).ok()
            }
        }
    };
}
kind!(
    ShardMessageKind,
    SHARD_REQ_PROTOCOL_VERSION,
    SHARD_RESP_PROTOCOL_VERSION
);
kind!(
    CdcMessageKind,
    CDC_REQ_PROTOCOL_VERSION,
    CDC_RESP_PROTOCOL_VERSION
);
kind!(
    RegistryMessageKind,
    REGISTRY_REQ_PROTOCOL_VERSION,
    REGISTRY_RESP_PROTOCOL_VERSION
);
kind!(
    BlocksMessageKind,
    BLOCKS_REQ_PROTOCOL_VERSION,
    BLOCKS_RESP_PROTOCOL_VERSION
);

/// A response body of some kind, or an error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply<K> {
    /// A response of `kind`; `body` is its encoding.
    Ok {
        /// Response kind.
        kind: K,
        /// Encoded response.
        body: Bytes,
    },
    /// An `ERROR` response.
    Err(ErrorCode),
}

/// A shard or CDC request frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpReq<K> {
    /// Request id, echoed by the response.
    pub request_id: u64,
    /// Request kind.
    pub kind: K,
    /// Encoded request.
    pub body: Bytes,
}

/// A shard or CDC response frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpResp<K> {
    /// Id of the request this answers.
    pub request_id: u64,
    /// Response or error.
    pub reply: Reply<K>,
}

/// A blocks request frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlocksReq {
    /// Block service the request is for.
    pub block_service_id: BlockServiceId,
    /// Request kind.
    pub kind: BlocksMessageKind,
    /// Encoded request.
    pub body: Bytes,
}

/// Size of a shard/CDC frame header.
pub const UDP_HEADER_LEN: usize = 4 + 8 + 1;
/// Size of a registry response header (protocol and length).
pub const REGISTRY_RESP_HEADER_LEN: usize = 4 + 4;
/// Size of a blocks response header (protocol and kind).
pub const BLOCKS_RESP_HEADER_LEN: usize = 4 + 1;

fn frame(header_len: usize, body: &impl Encode) -> Vec<u8> {
    Vec::with_capacity(header_len.saturating_add(body.encoded_len()))
}

fn check_protocol(buf: &mut Bytes, expected: u32) -> Result<(), FrameError> {
    let got = u32::decode(buf)?;
    if got == expected {
        Ok(())
    } else {
        Err(FrameError::Protocol { expected, got })
    }
}

fn kind_of<K: Kind>(v: u8) -> Result<K, FrameError> {
    K::from_u8(v).ok_or(FrameError::UnknownKind(v))
}

/// Response kind, or `None` for `ERROR`.
fn reply_kind<K: Kind>(v: u8) -> Result<Option<K>, FrameError> {
    if v == ERROR_KIND {
        Ok(None)
    } else {
        kind_of(v).map(Some)
    }
}

fn reply<K: Kind>(kind: Option<K>, body: Bytes) -> Result<Reply<K>, FrameError> {
    match kind {
        Some(kind) => Ok(Reply::Ok { kind, body }),
        None => Ok(Reply::Err(decode_exact(body)?)),
    }
}

fn put_reply<K: Kind>(out: &mut Vec<u8>, reply: &Reply<K>) -> Result<(), EncodeError> {
    match reply {
        Reply::Ok { kind, body } => {
            out.put_u8(kind.to_u8());
            out.put_slice(body);
            Ok(())
        }
        Reply::Err(code) => {
            out.put_u8(ERROR_KIND);
            code.encode(out)
        }
    }
}

// ---- shard / CDC ----

/// Encodes a shard/CDC request frame from its parts.
pub fn encode_udp_req<K: Kind>(req: &UdpReq<K>) -> Vec<u8> {
    let mut out = Vec::with_capacity(UDP_HEADER_LEN.saturating_add(req.body.len()));
    out.put_u32_le(K::REQ_PROTOCOL);
    out.put_u64_le(req.request_id);
    out.put_u8(req.kind.to_u8());
    out.put_slice(&req.body);
    out
}

/// Encodes a shard request.
pub fn encode_shard_req<R: ShardRequest>(request_id: u64, req: &R) -> Result<Vec<u8>, EncodeError> {
    let mut out = frame(UDP_HEADER_LEN, req);
    out.put_u32_le(SHARD_REQ_PROTOCOL_VERSION);
    out.put_u64_le(request_id);
    out.put_u8(R::KIND as u8);
    req.encode(&mut out)?;
    Ok(out)
}

/// Decodes a shard/CDC request frame (used by fake servers in tests).
pub fn decode_udp_req<K: Kind>(mut frame: Bytes) -> Result<UdpReq<K>, FrameError> {
    check_protocol(&mut frame, K::REQ_PROTOCOL)?;
    let request_id = u64::decode(&mut frame)?;
    let kind = kind_of(u8::decode(&mut frame)?)?;
    Ok(UdpReq {
        request_id,
        kind,
        body: frame,
    })
}

/// Encodes a shard/CDC response frame from its parts.
pub fn encode_udp_resp<K: Kind>(resp: &UdpResp<K>) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::with_capacity(UDP_HEADER_LEN.saturating_add(2));
    out.put_u32_le(K::RESP_PROTOCOL);
    out.put_u64_le(resp.request_id);
    put_reply(&mut out, &resp.reply)?;
    Ok(out)
}

/// Encodes a shard response (used by fake servers in tests).
pub fn encode_shard_resp<R: ShardResponse>(
    request_id: u64,
    resp: &R,
) -> Result<Vec<u8>, EncodeError> {
    let mut out = frame(UDP_HEADER_LEN, resp);
    out.put_u32_le(SHARD_RESP_PROTOCOL_VERSION);
    out.put_u64_le(request_id);
    out.put_u8(R::KIND as u8);
    resp.encode(&mut out)?;
    Ok(out)
}

/// Decodes a shard/CDC response frame.
pub fn decode_udp_resp<K: Kind>(mut frame: Bytes) -> Result<UdpResp<K>, FrameError> {
    check_protocol(&mut frame, K::RESP_PROTOCOL)?;
    let request_id = u64::decode(&mut frame)?;
    let kind = reply_kind(u8::decode(&mut frame)?)?;
    Ok(UdpResp {
        request_id,
        reply: reply(kind, frame)?,
    })
}

// ---- registry ----

/// Encodes a registry request.
pub fn encode_registry_req<R: RegistryRequest>(req: &R) -> Result<Vec<u8>, EncodeError> {
    encode_registry(REGISTRY_REQ_PROTOCOL_VERSION, R::KIND.to_u8(), req)
}

/// Encodes a registry response (used by fake servers in tests).
pub fn encode_registry_resp<R: RegistryResponse>(resp: &R) -> Result<Vec<u8>, EncodeError> {
    encode_registry(REGISTRY_RESP_PROTOCOL_VERSION, R::KIND.to_u8(), resp)
}

fn encode_registry(protocol: u32, kind: u8, body: &impl Encode) -> Result<Vec<u8>, EncodeError> {
    let len = u32::try_from(body.encoded_len().saturating_add(1))
        .map_err(|_| EncodeError::Invalid("registry message longer than 4 GiB"))?;
    let mut out = frame(REGISTRY_RESP_HEADER_LEN.saturating_add(1), body);
    out.put_u32_le(protocol);
    out.put_u32_le(len);
    out.put_u8(kind);
    body.encode(&mut out)?;
    Ok(out)
}

/// Encodes a registry response from its parts.
pub fn encode_registry_reply(reply: &Reply<RegistryMessageKind>) -> Result<Vec<u8>, EncodeError> {
    let mut data = Vec::new();
    put_reply(&mut data, reply)?;
    let len = u32::try_from(data.len())
        .map_err(|_| EncodeError::Invalid("registry message longer than 4 GiB"))?;
    let mut out = Vec::with_capacity(REGISTRY_RESP_HEADER_LEN.saturating_add(data.len()));
    out.put_u32_le(REGISTRY_RESP_PROTOCOL_VERSION);
    out.put_u32_le(len);
    out.put_slice(&data);
    Ok(out)
}

/// Checks a registry response header and returns the length of the data that follows,
/// rejecting lengths above `max` before anything is allocated.
pub fn registry_resp_len(
    header: [u8; REGISTRY_RESP_HEADER_LEN],
    max: usize,
) -> Result<usize, FrameError> {
    let mut buf = Bytes::copy_from_slice(&header);
    check_protocol(&mut buf, REGISTRY_RESP_PROTOCOL_VERSION)?;
    let len = u32::decode(&mut buf)?;
    match usize::try_from(len) {
        Ok(0) => Err(DecodeError::Truncated { needed: 1 }.into()),
        Ok(n) if n <= max => Ok(n),
        _ => Err(FrameError::TooLong { len, max }),
    }
}

/// Decodes the data of a registry response (kind byte, then body or error code).
pub fn decode_registry_data(mut data: Bytes) -> Result<Reply<RegistryMessageKind>, FrameError> {
    let kind = reply_kind(u8::decode(&mut data)?)?;
    reply(kind, data)
}

/// Decodes a whole registry response frame; the length must match exactly.
pub fn decode_registry_resp(
    mut frame: Bytes,
    max: usize,
) -> Result<Reply<RegistryMessageKind>, FrameError> {
    let header = crate::wire::take(&mut frame, REGISTRY_RESP_HEADER_LEN)?;
    let mut h = [0u8; REGISTRY_RESP_HEADER_LEN];
    h.copy_from_slice(&header);
    let len = registry_resp_len(h, max)?;
    if frame.len() < len {
        return Err(DecodeError::Truncated {
            needed: len.saturating_sub(frame.len()),
        }
        .into());
    }
    if frame.len() > len {
        return Err(DecodeError::TrailingBytes(frame.len().saturating_sub(len)).into());
    }
    decode_registry_data(frame)
}

/// Decodes a registry request frame (used by fake servers in tests).
pub fn decode_registry_req(mut frame: Bytes) -> Result<(RegistryMessageKind, Bytes), FrameError> {
    check_protocol(&mut frame, REGISTRY_REQ_PROTOCOL_VERSION)?;
    let len = u32::decode(&mut frame)?;
    let len = usize::try_from(len).unwrap_or(usize::MAX);
    if len != frame.len() || len == 0 {
        return Err(DecodeError::Truncated { needed: len }.into());
    }
    let kind = kind_of(u8::decode(&mut frame)?)?;
    Ok((kind, frame))
}

// ---- blocks ----

/// Encodes a blocks request.
pub fn encode_blocks_req<R: BlocksRequest>(
    block_service_id: BlockServiceId,
    req: &R,
) -> Result<Vec<u8>, EncodeError> {
    let mut out = frame(4 + 8 + 1, req);
    out.put_u32_le(BLOCKS_REQ_PROTOCOL_VERSION);
    block_service_id.encode(&mut out)?;
    out.put_u8(R::KIND.to_u8());
    req.encode(&mut out)?;
    Ok(out)
}

/// Encodes a blocks request frame from its parts.
pub fn encode_blocks_req_frame(req: &BlocksReq) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::with_capacity(13usize.saturating_add(req.body.len()));
    out.put_u32_le(BLOCKS_REQ_PROTOCOL_VERSION);
    req.block_service_id.encode(&mut out)?;
    out.put_u8(req.kind.to_u8());
    out.put_slice(&req.body);
    Ok(out)
}

/// Decodes a blocks request frame (used by fake servers in tests).
pub fn decode_blocks_req(mut frame: Bytes) -> Result<BlocksReq, FrameError> {
    check_protocol(&mut frame, BLOCKS_REQ_PROTOCOL_VERSION)?;
    let block_service_id = BlockServiceId::decode(&mut frame)?;
    let kind = kind_of(u8::decode(&mut frame)?)?;
    Ok(BlocksReq {
        block_service_id,
        kind,
        body: frame,
    })
}

/// Checks a blocks response header; `Ok(None)` means `ERROR` and a `u16` code follows.
pub fn blocks_resp_kind(
    header: [u8; BLOCKS_RESP_HEADER_LEN],
) -> Result<Option<BlocksMessageKind>, FrameError> {
    let mut buf = Bytes::copy_from_slice(&header);
    check_protocol(&mut buf, BLOCKS_RESP_PROTOCOL_VERSION)?;
    reply_kind(u8::decode(&mut buf)?)
}

/// Encodes the header of a blocks response of type `R`; the body (for
/// `FetchBlockWithCrc`, the pages) follows.
pub fn encode_blocks_resp<R: BlocksResponse>(resp: &R) -> Result<Vec<u8>, EncodeError> {
    let mut out = frame(BLOCKS_RESP_HEADER_LEN, resp);
    out.put_u32_le(BLOCKS_RESP_PROTOCOL_VERSION);
    out.put_u8(R::KIND.to_u8());
    resp.encode(&mut out)?;
    Ok(out)
}

/// Encodes a blocks response frame from its parts.
pub fn encode_blocks_reply(reply: &Reply<BlocksMessageKind>) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::new();
    out.put_u32_le(BLOCKS_RESP_PROTOCOL_VERSION);
    put_reply(&mut out, reply)?;
    Ok(out)
}

/// Decodes a whole blocks response frame held in memory.
pub fn decode_blocks_resp(mut frame: Bytes) -> Result<Reply<BlocksMessageKind>, FrameError> {
    check_protocol(&mut frame, BLOCKS_RESP_PROTOCOL_VERSION)?;
    let kind = reply_kind(u8::decode(&mut frame)?)?;
    reply(kind, frame)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)] // test code
mod tests {
    use super::*;
    use crate::generated::{FetchBlockWithCrcReq, LocalShardsReq, LookupReq, ROOT_DIR_INODE_ID};

    const GOLDENS: &str = include_str!("../../../testdata/goldens/frames.hex");
    const MAX: usize = 1 << 20;

    fn unhex(s: &str) -> Vec<u8> {
        if s == "-" {
            return Vec::new();
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// `(frame kind, bytes, description)` per golden line.
    fn goldens() -> Vec<(&'static str, Vec<u8>, &'static str)> {
        GOLDENS
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .map(|l| {
                let (frame, desc) = l.split_once(" # ").unwrap();
                let (kind, hex) = frame.split_once(' ').unwrap();
                (kind, unhex(hex), desc)
            })
            .collect()
    }

    /// Decodes with the decoder for `kind` and re-encodes with the matching encoder.
    fn roundtrip(kind: &str, bytes: &[u8]) -> Result<Vec<u8>, FrameError> {
        let b = Bytes::copy_from_slice(bytes);
        Ok(match kind {
            "shard_req" => encode_udp_req(&decode_udp_req::<ShardMessageKind>(b)?),
            "cdc_req" => encode_udp_req(&decode_udp_req::<CdcMessageKind>(b)?),
            "shard_resp" => encode_udp_resp(&decode_udp_resp::<ShardMessageKind>(b)?).unwrap(),
            "cdc_resp" => encode_udp_resp(&decode_udp_resp::<CdcMessageKind>(b)?).unwrap(),
            "registry_req" => {
                let (kind, body) = decode_registry_req(b)?;
                let mut out = Vec::new();
                out.put_u32_le(REGISTRY_REQ_PROTOCOL_VERSION);
                out.put_u32_le(u32::try_from(body.len() + 1).unwrap());
                out.put_u8(kind.to_u8());
                out.put_slice(&body);
                out
            }
            "registry_resp" => encode_registry_reply(&decode_registry_resp(b, MAX)?).unwrap(),
            "blocks_req" => encode_blocks_req_frame(&decode_blocks_req(b)?).unwrap(),
            "blocks_resp" => encode_blocks_reply(&decode_blocks_resp(b)?).unwrap(),
            other => panic!("unknown frame kind {other}"),
        })
    }

    #[test]
    fn goldens_round_trip() {
        let all = goldens();
        assert!(all.len() >= 12);
        for (kind, bytes, desc) in all {
            assert_eq!(roundtrip(kind, &bytes).unwrap(), bytes, "{kind}: {desc}");
        }
    }

    #[test]
    fn short_frames_rejected() {
        for (kind, bytes, desc) in goldens() {
            // Bodies of non-error replies are opaque here, so only the header (and, for
            // errors and registry frames, everything) must be complete.
            let header = match kind {
                "shard_req" | "cdc_req" | "shard_resp" | "cdc_resp" => UDP_HEADER_LEN,
                "blocks_req" => 13,
                "blocks_resp" => BLOCKS_RESP_HEADER_LEN,
                _ => bytes.len(),
            };
            let is_error = desc.contains("ERROR");
            let must_fail = if is_error { bytes.len() } else { header };
            for n in 0..must_fail {
                assert!(
                    roundtrip(kind, &bytes[..n]).is_err(),
                    "{kind} {desc}: prefix {n} accepted"
                );
            }
        }
    }

    #[test]
    fn typed_encoders_match_goldens() {
        let all = goldens();
        let lookup = LookupReq {
            dir_id: ROOT_DIR_INODE_ID,
            name: Bytes::from_static(b"hello.txt"),
        };
        assert_eq!(encode_shard_req(1, &lookup).unwrap(), all[0].1);
        assert_eq!(
            encode_registry_req(&LocalShardsReq {}).unwrap(),
            all.iter().find(|g| g.0 == "registry_req").unwrap().1
        );
        let fetch = FetchBlockWithCrcReq {
            block_id: crate::generated::BlockId(0xb10c),
            count: 2 * 4096,
            ..Default::default()
        };
        assert_eq!(
            encode_blocks_req(BlockServiceId(0x1000_0000_0000), &fetch).unwrap(),
            all.iter().find(|g| g.0 == "blocks_req").unwrap().1
        );
    }

    #[test]
    fn error_replies_decode_to_codes() {
        let all = goldens();
        let not_found = all.iter().find(|g| g.2.contains("NAME_NOT_FOUND")).unwrap();
        let resp =
            decode_udp_resp::<ShardMessageKind>(Bytes::copy_from_slice(&not_found.1)).unwrap();
        assert_eq!(resp.request_id, u64::MAX);
        assert_eq!(resp.reply, Reply::Err(ErrorCode::NAME_NOT_FOUND));
        // An error body must be exactly two bytes.
        let mut long = not_found.1.clone();
        long.push(0);
        assert!(decode_udp_resp::<ShardMessageKind>(Bytes::from(long)).is_err());
    }

    #[test]
    fn wrong_protocol_rejected() {
        for (kind, bytes, desc) in goldens() {
            let mut b = bytes.clone();
            b[0] ^= 0xff;
            assert!(
                matches!(roundtrip(kind, &b), Err(FrameError::Protocol { .. })),
                "{kind} {desc}"
            );
        }
        // Shard and CDC protocols are not interchangeable.
        let shard = &goldens()[1].1;
        assert!(decode_udp_resp::<CdcMessageKind>(Bytes::copy_from_slice(shard)).is_err());
    }

    #[test]
    fn unknown_kind_rejected() {
        let all = goldens();
        let mut shard = all[1].1.clone();
        shard[12] = 0xee;
        assert_eq!(
            decode_udp_resp::<ShardMessageKind>(Bytes::from(shard)),
            Err(FrameError::UnknownKind(0xee))
        );
        // ERROR is a response kind only.
        let mut req = all[0].1.clone();
        req[12] = ERROR_KIND;
        assert_eq!(
            decode_udp_req::<ShardMessageKind>(Bytes::from(req)),
            Err(FrameError::UnknownKind(ERROR_KIND))
        );
        let blocks = all.iter().find(|g| g.0 == "blocks_resp").unwrap();
        let mut h = [0u8; BLOCKS_RESP_HEADER_LEN];
        h.copy_from_slice(&blocks.1[..BLOCKS_RESP_HEADER_LEN]);
        h[4] = 0xee;
        assert_eq!(blocks_resp_kind(h), Err(FrameError::UnknownKind(0xee)));
    }

    #[test]
    fn registry_length_checked_before_allocation() {
        let mut h = [0u8; REGISTRY_RESP_HEADER_LEN];
        h[..4].copy_from_slice(&REGISTRY_RESP_PROTOCOL_VERSION.to_le_bytes());
        h[4..].copy_from_slice(&(MAX as u32 + 1).to_le_bytes());
        assert_eq!(
            registry_resp_len(h, MAX),
            Err(FrameError::TooLong {
                len: MAX as u32 + 1,
                max: MAX
            })
        );
        h[4..].copy_from_slice(&0u32.to_le_bytes());
        assert!(registry_resp_len(h, MAX).is_err());
        h[4..].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(registry_resp_len(h, MAX), Ok(7));
    }
}
