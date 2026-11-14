//! T109: decoding the largest golden `LocalFileSpansResp` and `ReadDirResp`.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // bench code

use bytes::Bytes;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use rustern_msgs::wire::{Decode, decode_exact};
use rustern_msgs::{LocalFileSpansResp, ReadDirResp};
use std::hint::black_box;

/// The longest encoding in `testdata/goldens/<name>.hex` (lines are `hex # comment`).
fn largest(name: &str) -> Bytes {
    let path = format!(
        "{}/../../testdata/goldens/{name}.hex",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(path).unwrap();
    let hex = text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next())
        .max_by_key(|h| h.len())
        .unwrap();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect::<Vec<u8>>();
    Bytes::from(bytes)
}

fn bench<T: Decode>(c: &mut Criterion, name: &str) {
    let input = largest(name);
    decode_exact::<T>(input.clone()).unwrap();
    let mut g = c.benchmark_group("decode");
    g.throughput(Throughput::Bytes(input.len() as u64));
    g.bench_function(name, |b| {
        b.iter(|| decode_exact::<T>(black_box(input.clone())).unwrap());
    });
    g.finish();
}

fn codec(c: &mut Criterion) {
    bench::<LocalFileSpansResp>(c, "LocalFileSpansResp");
    bench::<ReadDirResp>(c, "ReadDirResp");
}

criterion_group!(benches, codec);
criterion_main!(benches);
