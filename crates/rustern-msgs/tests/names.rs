//! Message kinds and error codes against upstream Go (`tools/goldens` → `names.txt`):
//! every wire value maps to the upstream name and back, and every other value is rejected.

#![allow(clippy::unwrap_used, clippy::panic)] // test code

use std::collections::BTreeMap;
use std::path::Path;

use bytes::Bytes;
use rustern_msgs::wire::{Encode, decode_exact, encode_to_vec};
use rustern_msgs::{
    BlocksMessageKind, CdcMessageKind, ErrorCode, RegistryMessageKind, ShardMessageKind,
};

/// `set -> (value -> NAME)` from `names.txt`.
fn upstream() -> BTreeMap<String, BTreeMap<u16, String>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/goldens/names.txt");
    let mut out: BTreeMap<String, BTreeMap<u16, String>> = BTreeMap::new();
    for line in std::fs::read_to_string(path).unwrap().lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut f = line.split(' ');
        let (set, value, name) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
        out.entry(set.to_owned())
            .or_default()
            .insert(value.parse().unwrap(), name.to_owned());
    }
    out
}

macro_rules! check_kinds {
    ($set:literal, $kind:ty) => {{
        let names = &upstream()[$set];
        assert_eq!(<$kind>::ALL.len(), names.len(), "{}: kind count", $set);
        for v in 0..=u8::MAX {
            match (<$kind>::try_from(v), names.get(&u16::from(v))) {
                (Ok(k), Some(name)) => {
                    assert_eq!(k.as_str(), name, "{} {v}", $set);
                    assert_eq!(k as u8, v);
                }
                (Err(e), None) => assert_eq!(e, v),
                (got, want) => panic!("{} {v}: Rust {got:?}, upstream {want:?}", $set),
            }
        }
        for (i, k) in <$kind>::ALL.iter().enumerate() {
            assert!(
                <$kind>::ALL[..i].iter().all(|p| (*p as u8) < (*k as u8)),
                "{}: ALL not in wire-value order",
                $set
            );
        }
    }};
}

#[test]
fn message_kinds_match_upstream() {
    check_kinds!("shard", ShardMessageKind);
    check_kinds!("cdc", CdcMessageKind);
    check_kinds!("registry", RegistryMessageKind);
    check_kinds!("blocks", BlocksMessageKind);
}

#[test]
fn error_codes_match_upstream() {
    let names = &upstream()["error"];
    for code in 0..=u16::MAX {
        let e = ErrorCode(code);
        match names.get(&code) {
            Some(name) => {
                assert_eq!(e.name(), Some(name.as_str()), "error {code}");
                assert_eq!(format!("{e:?}"), *name);
            }
            None => {
                assert_eq!(e.name(), None, "error {code}");
                assert_eq!(format!("{e:?}"), format!("ErrorCode({code})"));
            }
        }
        assert_eq!(e.to_string(), format!("{e:?}"));
        let bytes = encode_to_vec(&e).unwrap();
        assert_eq!(bytes, code.to_le_bytes());
        assert_eq!(e.encoded_len(), 2);
        assert_eq!(decode_exact::<ErrorCode>(Bytes::from(bytes)).unwrap(), e);
    }
}
