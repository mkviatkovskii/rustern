//! T109: block ranges of 4 KiB and 1 MiB reads in an RS(10,4) span (512,000-byte cells).
#![allow(missing_docs, clippy::unwrap_used, clippy::arithmetic_side_effects)] // bench code

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use rustern_core::layout::Geometry;
use std::hint::black_box;

fn layout(c: &mut Criterion) {
    let g = Geometry {
        data_blocks: 10,
        cell_size: 512_000,
        stripes: 1,
    };
    let mut group = c.benchmark_group("ranges");
    for len in [4u64 << 10, 1 << 20] {
        // Start mid-cell so the read crosses a cell boundary.
        let from = 512_000 - 4096;
        group.bench_with_input(BenchmarkId::from_parameter(len), &len, |b, len| {
            b.iter(|| g.ranges(black_box(from), black_box(from + len)).unwrap());
        });
    }
    group.finish();
}

criterion_group!(benches, layout);
criterion_main!(benches);
