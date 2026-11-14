//! T109: `crc-fast` (behind `rustern_core::crc`) against the `crc32c` crate (the earlier
//! choice, now a bench-only dev-dependency) at page, readahead and maximum-read sizes.
#![allow(missing_docs, clippy::unwrap_used)] // bench code

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

fn crc(c: &mut Criterion) {
    let mut g = c.benchmark_group("crc32c");
    for size in [4 << 10, 128 << 10, 1 << 20] {
        let data: Vec<u8> = (0..size).map(|i: usize| (i % 251) as u8).collect();
        assert_eq!(rustern_core::crc::crc32c(&data), crc32c::crc32c(&data));
        g.throughput(Throughput::Bytes(size as u64));
        g.bench_with_input(BenchmarkId::new("crc-fast", size), &data, |b, d| {
            b.iter(|| rustern_core::crc::crc32c(black_box(d)));
        });
        g.bench_with_input(BenchmarkId::new("crc32c", size), &data, |b, d| {
            b.iter(|| crc32c::crc32c(black_box(d)));
        });
    }
    g.finish();
}

criterion_group!(benches, crc);
criterion_main!(benches);
