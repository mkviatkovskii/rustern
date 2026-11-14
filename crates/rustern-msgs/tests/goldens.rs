//! Golden vectors from upstream Go (`tools/goldens`): every sample decodes, re-encodes
//! byte-identically, and every strict prefix fails to decode without panicking.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

use std::path::{Path, PathBuf};

use bytes::Bytes;
use rustern_msgs::wire::{Decode, Encode, decode_exact, encode_to_vec};
use rustern_msgs::*;

fn goldens_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/goldens")
}

/// `(hex bytes, description)` for every sample line of `file`.
fn samples(file: &Path) -> Vec<(Vec<u8>, String)> {
    let text = std::fs::read_to_string(file).unwrap();
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (hex, desc) = l.split_once(" # ").unwrap();
            (unhex(hex), desc.to_owned())
        })
        .collect()
}

fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn check<T: Encode + Decode + std::fmt::Debug>(bytes: &[u8], what: &str) {
    let value: T = decode_exact(Bytes::copy_from_slice(bytes))
        .unwrap_or_else(|e| panic!("{what}: decode failed: {e}"));
    assert_eq!(value.encoded_len(), bytes.len(), "{what}: encoded_len");
    assert_eq!(
        encode_to_vec(&value).unwrap(),
        bytes,
        "{what}: re-encode differs"
    );
    for n in 0..bytes.len() {
        assert!(
            decode_exact::<T>(Bytes::copy_from_slice(&bytes[..n])).is_err(),
            "{what}: prefix of {n} bytes decoded"
        );
    }
}

/// `check_as(type name, ...)` for every message struct; `MESSAGES` lists their names.
macro_rules! dispatch {
    ($($ty:ident),* $(,)?) => {
        const MESSAGES: &[&str] = &[$(stringify!($ty)),*];
        fn check_as(name: &str, bytes: &[u8], what: &str) {
            match name {
                $(stringify!($ty) => check::<$ty>(bytes, what),)*
                other => panic!("no Rust type for golden samples of {other}"),
            }
        }
    };
}
for_each_message!(dispatch);

/// Hand-picked samples: one `<Type>.hex` file per type.
#[test]
fn every_golden_round_trips() {
    let mut files: Vec<_> = std::fs::read_dir(goldens_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "hex"))
        .collect();
    files.sort();
    let mut checked = 0;
    for file in &files {
        let name = file.file_stem().unwrap().to_str().unwrap();
        if ["crc32c", "frames", "messages"].contains(&name) {
            continue;
        }
        for (bytes, desc) in samples(file) {
            check_as(name, &bytes, &format!("{name}: {desc}"));
            checked += 1;
        }
    }
    assert!(checked >= 30, "only {checked} golden samples found");
}

/// Reflection-filled samples of every upstream struct (`messages.hex`, `<Type> <hex>`):
/// every Rust message type has at least one.
#[test]
fn every_message_type_round_trips() {
    let text = std::fs::read_to_string(goldens_dir().join("messages.hex")).unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let (sample, desc) = line.split_once(" # ").unwrap();
        let (name, hex) = sample.split_once(' ').unwrap();
        check_as(name, &unhex(hex), &format!("{name}: {desc}"));
        seen.insert(name);
    }
    let missing: Vec<_> = MESSAGES.iter().filter(|m| !seen.contains(*m)).collect();
    assert!(missing.is_empty(), "no golden samples for {missing:?}");
}
