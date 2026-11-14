//! Entry points shared by the fuzz targets (`fuzz/`) and the stable-Rust corpus replay
//! test. Each decodes a response frame and, for kinds the client reads, its body; whatever
//! decodes must re-encode to bytes that decode to an equal value. Panics mean a bug.

#![allow(clippy::panic, clippy::expect_used)] // panicking is how a fuzz target reports

use std::fmt::Debug;

use bytes::Bytes;

use crate::frame::{
    Reply, decode_blocks_resp, decode_registry_resp, decode_udp_resp, encode_blocks_reply,
    encode_registry_reply, encode_udp_resp,
};
use crate::generated::{
    BlocksMessageKind, FetchBlockWithCrcResp, FileSpansResp, LocalFileSpansResp, LocalShardsResp,
    LookupResp, ReadDirResp, RegistryMessageKind, ShardMessageKind, StatDirectoryResp,
    StatFileResp,
};
use crate::wire::{Decode, Encode, decode_exact, encode_to_vec};

/// Largest registry response the client accepts (same limit as `rustern-client`).
const REGISTRY_MAX: usize = 1 << 20;

fn body<T: Encode + Decode + PartialEq + Debug>(body: Bytes) {
    let Ok(v) = decode_exact::<T>(body.clone()) else {
        return;
    };
    let enc = encode_to_vec(&v).expect("decoded value must re-encode");
    assert_eq!(
        enc.len(),
        v.encoded_len(),
        "encoded_len disagrees with encode"
    );
    assert_eq!(enc, body, "re-encoding changed the bytes");
    let again = decode_exact::<T>(Bytes::from(enc)).expect("re-encoded value must decode");
    assert_eq!(v, again);
}

/// Shard response frame.
pub fn shard_resp(data: &[u8]) {
    let Ok(resp) = decode_udp_resp::<ShardMessageKind>(Bytes::copy_from_slice(data)) else {
        return;
    };
    assert_eq!(encode_udp_resp(&resp).expect("frame re-encodes"), data);
    if let Reply::Ok { kind, body: b } = resp.reply {
        match kind {
            ShardMessageKind::Lookup => body::<LookupResp>(b),
            ShardMessageKind::StatFile => body::<StatFileResp>(b),
            ShardMessageKind::StatDirectory => body::<StatDirectoryResp>(b),
            ShardMessageKind::ReadDir => body::<ReadDirResp>(b),
            ShardMessageKind::LocalFileSpans => body::<LocalFileSpansResp>(b),
            ShardMessageKind::FileSpans => body::<FileSpansResp>(b),
            _ => {}
        }
    }
}

/// Registry response frame.
pub fn registry_resp(data: &[u8]) {
    let Ok(reply) = decode_registry_resp(Bytes::copy_from_slice(data), REGISTRY_MAX) else {
        return;
    };
    assert_eq!(
        encode_registry_reply(&reply).expect("frame re-encodes"),
        data
    );
    if let Reply::Ok {
        kind: RegistryMessageKind::LocalShards,
        body: b,
    } = reply
    {
        body::<LocalShardsResp>(b);
    }
}

/// Blocks response frame. For `FetchBlockWithCrc` the body is the empty response
/// followed by pages, which are opaque here.
pub fn blocks_resp(data: &[u8]) {
    let Ok(reply) = decode_blocks_resp(Bytes::copy_from_slice(data)) else {
        return;
    };
    assert_eq!(encode_blocks_reply(&reply).expect("frame re-encodes"), data);
    if let Reply::Ok {
        kind: BlocksMessageKind::FetchBlockWithCrc,
        body: mut b,
    } = reply
    {
        let _ = FetchBlockWithCrcResp::decode(&mut b);
    }
}
